#!/usr/bin/env python3
"""Authored boundary regressions; NOT another independent model quality test."""
from __future__ import annotations

import argparse
import json
import sys
import tempfile
from dataclasses import replace
from pathlib import Path
from collections.abc import Iterable, Sequence
from unittest.mock import patch

from auxiliary_runtime_evidence import packaged_intent, runtime_provenance
from keyswitch.backend import KeyEvent
from keyswitch.boundary_model import BoundaryModel
from keyswitch.boundary_policy import ARTIFACT, BoundaryPolicy
from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from context_corpus import ROOT
from context_evidence import canonical, checksum
# The engine serves the onboard lexicon plus the packaged supplement, so the replay does too.
from reference_lexicon import reference_models
from train_boundary_model import CANDIDATE, DIRECTORY
from verify_lexical_compatibility import verify as verify_compatibility

if str(ROOT / "tests") not in sys.path:
    sys.path.insert(0, str(ROOT / "tests"))
from test_input_integrity import EditorBackend


REPORT = DIRECTORY / "engine-regression.json"
INITIAL_KEY_SERIAL = 100
SCENARIOS = (
    ("ghj,ktvf", "проблема"), ("ghtlkj;bk", "предложил"), (",b,kbjntrf", "библиотека"),
    ("rjnjhe.", "которую"), ("rjnjhe.,", "которую,"), ("ghbdtn,", "привет,"),
    ("ghbdtn.", "привет."), ("ghbdtn...", "привет..."), ("ghbdtn;", "привет;"),
    ("hello,", "hello,"), ("hello...", "hello..."), ("hello,world", "hello,world"),
    ("don't", "don't"), ("example.org", "example.org"), ("path/file.py", "path/file.py"),
    ("pm2", "pm2"), ("HTTP", "HTTP"), ("we’re", "we’re"),
)


def provenance() -> dict[str, str]:
    return runtime_provenance(ROOT, [Path(__file__), CANDIDATE, ARTIFACT,
        ROOT / "model/boundary_v2/corpus.json", ROOT / "model/boundary_v2/seal.json",
        ROOT / "model/boundary_v2/lexical-compatibility.json", ROOT / "tools/train_boundary_model.py",
        ROOT / "tools/verify_lexical_compatibility.py",
        ROOT / "src/keyswitch/resources/lexicon-supplement-ru_RU.json",
        ROOT / "model/intent_v1/compatibility/generation-config-v21.json"])


def replay(original: str, model: BoundaryModel | None, models: dict[int, LanguageModel]) -> tuple[str, int]:
    pair = LayoutPair()
    with tempfile.TemporaryDirectory(prefix="keyswitch-boundary-replay-") as temporary:
        root = Path(temporary)
        settings = SettingsStore(root / "config.json")
        settings.set("detection.context_policy", "off")
        settings.set("detection.early_switch", False)
        settings.set("detection.learning", False)
        settings.set("general.keep_history", False)
        backend = EditorBackend()
        def load(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
            return models[0 if locale == "en_US" else 1]
        with patch("keyswitch.engine.LanguageModel.load", side_effect=load), \
                patch("keyswitch.engine.LinearNgramModel.try_load_default", return_value=packaged_intent(ROOT)):
            engine = KeySwitchEngine(settings, HistoryStore(root / "history.jsonl"), backend)
        engine.boundary_model = model
        early = 0
        for serial, char in enumerate(original + " ", INITIAL_KEY_SERIAL):
            characters = (char, pair.translate(char, "us", "ru"))
            observed = characters[backend.group]
            event = KeyEvent(True, serial, "space" if observed == " " else observed, observed, characters, backend.group, 0, serial)
            backend.type(event)
            engine._handle(event)
            engine._handle(replace(event, pressed=False))
            if char != " ":
                early = len(backend.injections)
        return backend.text, early


def evaluate() -> bytes:
    verify_compatibility("boundary_v2")
    runtime = provenance()
    models = reference_models(False)
    results: dict[str, object] = {}
    for label, model in (("legacy_no_segmentation", None), ("rejected_v1", BoundaryModel.load(CANDIDATE)),
                         ("active_v2", BoundaryPolicy.load())):
        rows: list[dict[str, object]] = []
        exact, changed_correct, premature, mismatched_length = 0, 0, 0, 0
        for original, expected in SCENARIOS:
            actual, early = replay(original, model, models)
            matches = actual == expected + " "
            exact += matches
            changed_correct += original == expected and not matches
            premature += early
            mismatched_length += len(actual) != len(expected) + 1
            rows.append({"original": original, "expected": expected + " ", "actual": actual, "before_hard_boundary": early})
        results[label] = {"rows": rows, "exact": exact, "changed_correct": changed_correct,
                          "injections_before_hard_boundary": premature, "length_mismatches": mismatched_length}
    if provenance() != runtime:
        raise ValueError("boundary runtime inputs changed during replay")
    return canonical({"schema_version": 1, "scope": "authored in-process regression; fixed physical keys/live layout; no native OS proof or independent model evaluation",
                      "provenance": runtime, "results": results})


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--refresh-runtime", action="store_true")
    args = parser.parse_args(argv)
    if args.verify and args.refresh_runtime:
        parser.error("choose verify or runtime refresh")
    content = evaluate()
    if args.verify:
        if REPORT.read_bytes() != content:
            raise ValueError("boundary engine regression changed")
    elif REPORT.exists() and not args.refresh_runtime:
        raise ValueError("regression report already exists; use --verify")
    else:
        if REPORT.exists():
            history = DIRECTORY / "engine-history"
            history.mkdir(exist_ok=True)
            (history / (checksum(REPORT) + ".json")).write_bytes(REPORT.read_bytes())
        REPORT.write_bytes(content)
    result = json.loads(content)
    for name, metrics in result["results"].items():
        print(name, json.dumps({key: value for key, value in metrics.items() if key != "rows"}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
