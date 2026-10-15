#!/usr/bin/env python3
"""The owner's own typing as a release barrier: no word the released version got right may come out wrong.

The tests in tests/ hold every case written down so far, and a sealed test measures text nobody tuned on; neither holds
the owner's own typing, which the field batteries rebuild from the technical logs the owner sends (a stream of words
per message, each with the reading the log implies: what the engine did and how the owner reacted). From 15.10.2026,
by the owner's decision, every release - a new model pair or a change of the engine - is replayed on every battery
twice: with the whole tree of the last release (its engine and its models) and with the tree to be released. A word
the released tree got right that the new tree gets wrong blocks the release until it is fixed in the new tree or, where
the reading the log implies is wrong (the owner left garbage uncorrected in a prompt), a verdict says what the word
should have been.

Verdicts are append-only records in the private verdicts file, each with who gave it (`llm` or `owner`), when, a
reason code and, for a new one, the reason in words. A word the owner labelled by hand (a Pause, a manual switch,
retyping it) is the owner's: only an `owner` verdict may change it, and an `llm` verdict that would - another reading,
or taking it out as `ambiguous` - is set aside, listed among the words that need the owner, and the word counts with
the owner's own label. A verdict of `ambiguous` takes any other word out until the owner rules. The record of every run lists each verdict that decided a word against the reading
the log implies, and how many of them are newer than the release compared with, so the owner sees every one.

The batteries, the verdicts, the replay script and the records are private (a manifest under .t/ names them, and the
records must lie where git ignores them). This tool prints counts alone, even when it fails; the words go into the
private record.

    python3 tools/field_regression_gate.py [--manifest .t/field/manifest.json] [--candidate-root DIR] [--released-ref TAG]

Exit status: 0 when no word broke, 1 when some did, FIELD_GATE_ERROR_EXIT_CODE when the gate could not run.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import io
import json
import os
import subprocess
import sys
import tarfile
from collections.abc import Callable, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from typing import Final, cast

ROOT: Final = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from keyswitch.constants.release import (  # noqa: E402
    FIELD_GATE_ERROR_EXIT_CODE,
    FIELD_GATE_RECORD_FILE_MODE,
    FIELD_GATE_REPLAY_TIMEOUT_SECONDS,
    FIELD_GATE_WORKERS,
)

# The private manifest; KEYSWITCH_FIELD_MANIFEST names another.
DEFAULT_MANIFEST: Final = ROOT / ".t/field/manifest.json"
MANIFEST_VARIABLE: Final = "KEYSWITCH_FIELD_MANIFEST"
KEEP: Final = "keep"
CONVERT: Final = "convert"
# Verdicts that take a word out of the comparison: nobody can tell its reading, or only a part of it converts.
EXCLUDED_VERDICTS: Final = frozenset({"ambiguous", "partial"})
VERDICTS: Final = frozenset({KEEP, CONVERT}) | EXCLUDED_VERDICTS
OWNER: Final = "owner"
AUTHORS: Final = frozenset({"llm", OWNER})
# Why the reading the log implies was wrong. The review of 15.10.2026, before this gate, recorded no reason per word.
REVIEW_BEFORE_GATE: Final = "review-2026-10-15"
REASON_CODES: Final = frozenset({"garbage-in-prompt", "code-identifier", "retyped-elsewhere", "russian-typo",
                                 "owner-decision", REVIEW_BEFORE_GATE})
# Variables the replay script reads that would swap the tree's own models or settings for others: every `FIELD_*` one is
# cleared, then the manifest's settings are set; no display reaches a replay.
REPLAY_VARIABLE_PREFIX: Final = "FIELD_"
DISPLAY_VARIABLES: Final = ("DISPLAY", "WAYLAND_DISPLAY")
RELEASED: Final = "released"
CANDIDATE: Final = "candidate"
TREE_MARKER: Final = ".field-gate-commit"
SUMMARY_PREFIX: Final = "field regression gate:"
# A verdict's key: battery, stream, the word's time and its typed reading.
VERDICT_KEY_FIELDS: Final = ("battery", "stream", "time", "typed")


class GateError(Exception):
    """The gate could not run; the message names files and counts, never a word."""


@dataclass(frozen=True)
class Manifest:
    batteries: dict[str, Path]
    verdicts: Path
    replay: Path
    records: Path
    environment: dict[str, str]


@dataclass(frozen=True)
class Row:
    """One word of a battery as a tree typed it."""

    stream: int
    index: int
    time: str
    typed: str
    other: str
    intended: str
    shown: str
    owner_labelled: bool
    version: str


@dataclass
class Comparison:
    rows: int = 0
    excluded: int = 0
    released_right: int = 0
    candidate_right: int = 0
    broken: list[dict[str, object]] = field(default_factory=list)
    fixed: list[dict[str, object]] = field(default_factory=list)
    verdicts_used: list[dict[str, object]] = field(default_factory=list)
    new_verdicts_used: int = 0
    needs_owner: list[dict[str, object]] = field(default_factory=list)

    def counts(self) -> dict[str, int]:
        return {"rows": self.rows, "excluded": self.excluded, "released_right": self.released_right,
                "candidate_right": self.candidate_right, "fixed": len(self.fixed), "broken": len(self.broken),
                "verdicts_used": len(self.verdicts_used), "new_verdicts_used": self.new_verdicts_used,
                "needs_owner": len(self.needs_owner)}


def load_manifest(path: Path, root: Path = ROOT) -> Manifest:
    try:
        payload = cast(dict[str, object], json.loads(path.read_text("utf-8")))
    except FileNotFoundError:
        raise GateError(f"no field manifest at {path}: the owner's batteries are not on this machine") from None
    batteries = {str(name): root / str(value) for name, value in sorted(cast(dict[str, object], payload["batteries"]).items())}
    if not batteries:
        raise GateError("the field manifest names no battery")
    return Manifest(batteries, root / str(payload["verdicts"]), root / str(payload["replay"]), root / str(payload["records"]),
                    {str(key): str(value) for key, value in cast(dict[str, object], payload.get("environment", {})).items()})


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(root: Path, *arguments: str) -> str:
    return subprocess.run(["git", *arguments], cwd=root, check=True, capture_output=True, text=True).stdout.strip()


def latest_release(root: Path = ROOT) -> str:
    """The tag of the last release: the newest `v*` tag reachable from HEAD."""

    return git(root, "describe", "--tags", "--abbrev=0", "--match", "v*")


def ensure_ignored(path: Path, root: Path = ROOT) -> None:
    """The records name the owner's words: they may only lie where git ignores them (a release commits `git add -A`)."""

    if subprocess.run(["git", "check-ignore", "-q", str(path)], cwd=root, check=False).returncode != 0:
        raise GateError(f"{path} is not ignored by git; the gate writes the owner's words only where git ignores them")


def release_tree(tag: str, records: Path, root: Path = ROOT) -> Path:
    """The tag's tracked files under the private records, exported once with `git archive` (no worktree is added to
    the repository while a release may be running) and marked with the commit they came from."""

    commit = git(root, "rev-list", "-n", "1", tag)
    path = records / "trees" / tag
    marker = path / TREE_MARKER
    if marker.is_file() and marker.read_text("utf-8").strip() == commit:
        return path
    if path.exists():
        raise GateError(f"{path} exists but is no export of {tag}; remove it")
    archive = subprocess.run(["git", "archive", "--format=tar", commit], cwd=root, check=True, capture_output=True).stdout
    path.mkdir(parents=True)
    with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
        bundle.extractall(path, filter="data")
    marker.write_text(commit + "\n", encoding="utf-8")
    return path


def replay_environment(tree: Path, manifest: Manifest) -> dict[str, str]:
    environment = {name: value for name, value in os.environ.items()
                   if not name.startswith(REPLAY_VARIABLE_PREFIX) and name not in DISPLAY_VARIABLES}
    environment.update(manifest.environment)
    environment["PYTHONPATH"] = os.pathsep.join(str(tree / part) for part in ("src", "tools", "tests"))
    return environment


def replay(tree: Path, battery: Path, output: Path, manifest: Manifest) -> None:
    """One battery typed through the engine, models and harness of `tree` (its own directory is the working one), in a
    private session bus of its own; what the script prints stays in a private log beside its report."""

    output.parent.mkdir(parents=True, exist_ok=True)
    with output.with_suffix(".log").open("w", encoding="utf-8") as log:
        completed = subprocess.run(
            ["dbus-run-session", "--", sys.executable, str(manifest.replay), "replay", str(battery), str(output)],
            cwd=tree, env=replay_environment(tree, manifest), stdout=log, stderr=subprocess.STDOUT,
            timeout=FIELD_GATE_REPLAY_TIMEOUT_SECONDS, check=False)
    if completed.returncode != 0 or not output.is_file():
        raise GateError(f"the replay of battery {battery.name} in {tree} failed (exit {completed.returncode}); "
                        f"its log is {output.with_suffix('.log')}")


def read_rows(report: Path) -> dict[tuple[int, int], Row]:
    try:
        payload = cast(dict[str, object], json.loads(report.read_text("utf-8")))
        raw = cast(list[list[object]], payload["rows"])
    except (json.JSONDecodeError, KeyError, TypeError):
        raise GateError(f"{report.name} is no replay report with rows") from None
    rows: dict[tuple[int, int], Row] = {}
    for stream, index, time, typed, other, intended, shown, owner, version in raw:
        row = Row(int(cast(int, stream)), int(cast(int, index)), str(time), str(typed), str(other), str(intended), str(shown),
                  bool(owner), str(version))
        if (row.stream, row.index) in rows:
            raise GateError(f"{report.name} holds one word twice")
        rows[row.stream, row.index] = row
    return rows


def read_verdicts(path: Path) -> dict[tuple[str, int, str], dict[str, object]]:
    """Verdicts by battery, stream and the word's time (the keys are `battery|stream|time|typed`), each checked for an
    author, a date and a reason."""

    verdicts: dict[tuple[str, int, str], dict[str, object]] = {}
    for key, value in cast(dict[str, dict[str, object]], json.loads(path.read_text("utf-8"))).items():
        battery, stream, time, _typed = key.split("|", len(VERDICT_KEY_FIELDS) - 1)
        if (value.get("verdict") not in VERDICTS or value.get("by") not in AUTHORS or not value.get("date")
                or value.get("reason_code") not in REASON_CODES
                or value.get("reason_code") != REVIEW_BEFORE_GATE and not value.get("reason")):
            raise GateError(f"a verdict of battery {battery}, stream {stream} lacks its verdict, author, date or reason")
        verdicts[battery, int(stream), time] = value
    return verdicts


def target(row: Row, verdict: Mapping[str, object] | None) -> str | None:
    """The text the word should end as: the verdict's when there is one, else the reading the log implies; None when
    the verdict takes the word out. An `llm` verdict on a word the owner labelled otherwise counts for nothing
    (owner_overruled)."""

    if verdict is None or owner_overruled(row, verdict):
        return row.intended
    name = verdict["verdict"]
    if name in EXCLUDED_VERDICTS:
        return None
    marked = str(verdict["typed"])
    return marked if name == KEEP else row.other if marked == row.typed else row.typed


def owner_overruled(row: Row, verdict: Mapping[str, object]) -> bool:
    """Whether an `llm` verdict would change a word the owner labelled by hand - another reading, or taking it out as
    `ambiguous`: only the owner may."""

    marked = str(verdict["typed"])
    if marked not in (row.typed, row.other):
        raise GateError(f"a verdict of stream {row.stream} names neither reading of its word")
    if not row.owner_labelled or verdict["by"] == OWNER:
        return False
    if verdict["verdict"] in EXCLUDED_VERDICTS:
        return True
    goal = marked if verdict["verdict"] == KEEP else row.other if marked == row.typed else row.typed
    return goal != row.intended


def compare(released: Mapping[str, Mapping[tuple[int, int], Row]], candidate: Mapping[str, Mapping[tuple[int, int], Row]],
            verdicts: Mapping[tuple[str, int, str], Mapping[str, object]], since: str = "") -> Comparison:
    """Every word of every battery as both trees typed it, against its target. A broken word notes whether it is the
    first word of its stream the trees typed differently: the words after it may only follow from it. A verdict dated
    `since` or later counts as new."""

    result = Comparison()
    for battery in sorted(released):
        before, after = released[battery], candidate[battery]
        if before.keys() != after.keys():
            raise GateError(f"the trees typed different words of battery {battery}")
        diverged: set[int] = set()
        for key in sorted(before):
            old, new = before[key], after[key]
            verdict = verdicts.get((battery, old.stream, old.time))
            goal = target(old, verdict)
            if verdict is not None and owner_overruled(old, verdict):
                result.needs_owner.append({"battery": battery, "stream": old.stream, "time": old.time, "typed": old.typed,
                                           "implied": old.intended, "verdict": verdict["verdict"], "by": verdict["by"]})
                verdict = None
            first = old.shown != new.shown and old.stream not in diverged
            if old.shown != new.shown:
                diverged.add(old.stream)
            if goal is None:
                result.excluded += 1
                continue
            result.rows += 1
            right = (old.shown == goal, new.shown == goal)
            result.released_right += right[0]
            result.candidate_right += right[1]
            word = {"battery": battery, "stream": old.stream, "index": old.index, "time": old.time, "typed": old.typed,
                    "target": goal, "released": old.shown, "candidate": new.shown, "owner_labelled": old.owner_labelled,
                    "logged_by": old.version, "first_divergence": first}
            if verdict is not None and ((old.shown == old.intended) != right[0] or (new.shown == new.intended) != right[1]):
                result.verdicts_used.append({**word, "implied": old.intended,
                                             **{name: verdict.get(name) for name in ("verdict", "by", "date", "reason_code", "reason")}})
                result.new_verdicts_used += str(verdict["date"]) >= since if since else 0
            if right == (True, False):
                result.broken.append(word)
            elif right == (False, True):
                result.fixed.append(word)
    return result


def replays(trees: Mapping[str, Path], manifest: Manifest, directory: Path, cached: Callable[[str, str], Path | None],
            run: Callable[[Path, Path, Path, Manifest], None] = replay, workers: int = FIELD_GATE_WORKERS,
            ) -> dict[str, dict[str, dict[tuple[int, int], Row]]]:
    """Every battery replayed in every tree, `workers` at a time; a replay `cached` names is read, not run again."""

    jobs: list[tuple[str, str, Path]] = []
    reports: dict[tuple[str, str], Path] = {}
    for side in trees:
        for name in manifest.batteries:
            known = cached(side, name)
            reports[side, name] = known if known is not None else directory / side / f"{name}.json"
            if known is None:
                jobs.append((side, name, reports[side, name]))
    with ThreadPoolExecutor(max_workers=workers) as pool:
        for future in [pool.submit(run, trees[side], manifest.batteries[name], output, manifest) for side, name, output in jobs]:
            future.result()
    return {side: {name: read_rows(reports[side, name]) for name in manifest.batteries} for side in trees}


def released_cache(manifest: Manifest, commit: str) -> Callable[[str, str], Path]:
    """The released tree never changes: its replay of a battery is kept by the tag's commit, the interpreter and the
    bytes of the battery, the replay script and its settings."""

    settings = json.dumps(manifest.environment, sort_keys=True)

    def path(side: str, name: str) -> Path:
        parts = (side, commit, sys.version, checksum(manifest.batteries[name]), checksum(manifest.replay), settings)
        key = hashlib.sha256("\0".join(parts).encode()).hexdigest()
        return manifest.records / "released" / f"{name}-{key}.json"
    return path


def private_write(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, FIELD_GATE_RECORD_FILE_MODE)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        handle.write(json.dumps(payload, ensure_ascii=False, indent=1) + "\n")


def run_gate(manifest: Manifest, candidate_root: Path, tag: str, *, root: Path = ROOT,
             run: Callable[[Path, Path, Path, Manifest], None] = replay,
             now: Callable[[], datetime.datetime] = lambda: datetime.datetime.now(datetime.timezone.utc),
             ) -> tuple[Comparison, Path]:
    ensure_ignored(manifest.records, root)
    stamp = now().strftime("%Y%m%dT%H%M%SZ")
    commit = git(root, "rev-list", "-n", "1", tag)
    since = git(root, "log", "-1", "--format=%cs", commit)
    released = release_tree(tag, manifest.records, root)
    cache = released_cache(manifest, commit)
    directory = manifest.records / "runs" / stamp
    names = {path: name for name, path in manifest.batteries.items()}

    def cached(side: str, name: str) -> Path | None:
        path = cache(side, name)
        return path if side == RELEASED and path.is_file() else None

    def keeping(tree: Path, battery: Path, output: Path, manifest: Manifest) -> None:
        run(tree, battery, output, manifest)
        if tree == released:
            kept = cache(RELEASED, names[battery])
            kept.parent.mkdir(parents=True, exist_ok=True)
            kept.write_bytes(output.read_bytes())

    rows = replays({RELEASED: released, CANDIDATE: candidate_root}, manifest, directory, cached, keeping)
    comparison = compare(rows[RELEASED], rows[CANDIDATE], read_verdicts(manifest.verdicts), since)
    record = manifest.records / "gate" / f"{stamp}-{tag}.json"
    private_write(record, {
        "released": {"tag": tag, "commit": commit, "date": since}, "candidate": str(candidate_root),
        "counts": comparison.counts(), "batteries": {name: checksum(path) for name, path in manifest.batteries.items()},
        "verdicts_sha256": checksum(manifest.verdicts), "replay_sha256": checksum(manifest.replay),
        "interpreter": sys.version, "environment": manifest.environment, "settings": "the defaults of each tree",
        "broken": comparison.broken, "fixed": comparison.fixed, "verdicts_used": comparison.verdicts_used,
        "needs_owner": comparison.needs_owner})
    return comparison, record


def summary(comparison: Comparison, tag: str) -> str:
    counts = comparison.counts()
    return SUMMARY_PREFIX + " " + ", ".join(f"{name}={value}" for name, value in counts.items()) + f" (against {tag})"


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--manifest", type=Path, default=Path(os.environ.get(MANIFEST_VARIABLE, DEFAULT_MANIFEST)))
    parser.add_argument("--candidate-root", type=Path, default=ROOT, help="the tree to be released (default: this one)")
    parser.add_argument("--released-ref", default=None, help="the release to compare with (default: the last v* tag)")
    arguments = parser.parse_args(argv)
    try:
        manifest = load_manifest(arguments.manifest)
        tag = arguments.released_ref or latest_release()
        comparison, record = run_gate(manifest, arguments.candidate_root.resolve(), tag)
    except Exception as error:  # noqa: BLE001 - the gate fails closed and names no word, whatever went wrong
        detail = f": {error}" if isinstance(error, GateError) else ""
        print(f"{SUMMARY_PREFIX} could not run: {type(error).__name__}{detail}", flush=True)
        return FIELD_GATE_ERROR_EXIT_CODE
    print(summary(comparison, tag), flush=True)
    print(f"{SUMMARY_PREFIX} record {record}", flush=True)
    if comparison.broken:
        print(f"{SUMMARY_PREFIX} FAILED: {len(comparison.broken)} words the released version got right come out wrong", flush=True)
        return 1
    print(f"{SUMMARY_PREFIX} passed", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
