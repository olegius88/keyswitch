#!/usr/bin/env python3
"""The sealed test of developer chat: fresh ru.stackoverflow comments typed through the engine, each read once.

The owner's decisions of 15.10.2026: besides the Tatoeba sequences (evaluate_context_action_sequences), a second sealed
test from developer chat - comments of ru.stackoverflow written in 2024-2026, after every source a model of KeySwitch
was fitted on (tools/build_devchat_messages.py) - read only for a release candidate whose Tatoeba test passed and whose
development gain was recorded first, with two floors:
- F1, better than the baseline pair: net = ok(forget_ru) + ok(forget_en) + ok(start_wrong) - corruptions(correct) at
  least the baseline pair's net + DEVCHAT_MIN_NET_MARGIN, corruptions no more than its;
- F3, not worse than the released pair, judged pair by pair on the same messages: in each mode the messages the
  candidate loses and the released pair gets right may outnumber those it gains only as chance would (a one-sided
  binomial test of the losses among the messages the two pairs disagree on, DEVCHAT_SIGNIFICANCE); the three modes
  together restore no fewer messages than the released pair; corruptions no more than its.
Every pair must type every message: a message a pair could not type fails the floors.

The bench is mixed_typing's: each message typed with the keys of its intended layout, the layout chosen by hand where the
language changes, in four modes (every change made; one change to Russian forgotten; one to English forgotten; the
message started in the other layout). The field is read, a layout chosen by hand is respected, the early switch is off:
the pairs differ in their context model alone.

A read of the test claims its messages and questions in a ledger of its own (DEVCHAT_LEDGER) before any text is opened:
the messages are drawn from the members file (the SHA-256 of each text, its question and its mix), a read that overlaps
the messages or questions of an earlier one is refused, and so is a second read for the same candidate - an existing
result is reported again instead. The development split is read freely; its result for a candidate is recorded once
(`<artifact sha256>.dev.json`) with the runtime it ran on, and the test is read only after that record shows both floors
held, on the same runtime, and after the candidate passed its Tatoeba test. Results hold counts only and go to files
only their owner reads; no message is printed or written anywhere, and a failure names its kind alone.

Every read takes whole discussions out of the test (a message's question goes with it), so the test is used up: on
19.10.2026, after four reads, no message with Latin words was left, and the fifth read drew none. The test therefore
grows by batches: later batches are built by the same builder from fresh comments of the same months that the earlier
fetches did not take, each pinned in TEST_BATCHES by the SHA-256 of its messages, and a read draws from every batch.
A read that would take fewer messages with Latin words than DEVCHAT_TEST_COUNT is refused before it claims anything (a
new batch is needed), and an earlier claim that drew no message read no test: it is no read of its candidate.

    PYTHONPATH=src:tools:tests dbus-run-session -- python3 tools/evaluate_devchat_test.py --split dev|test \\
        --directory <messages directory> --candidate <context artifact> --released <artifact> --output <result.json>
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import sys
from collections.abc import Mapping, Sequence
from concurrent.futures import ProcessPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import Final, cast

ROOT: Final = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "src"), str(ROOT / "tests"), str(ROOT / "tools")]

from keyswitch.constants.corpus import (  # noqa: E402
    DEVCHAT_DEV_CONTENT_SHA256, DEVCHAT_NAMESPACE, DEVCHAT_SEED, DEVCHAT_TEST_CONTENT_SHA256, DEVCHAT_TEST_COUNT,
    MIXED_EVALUATE_RUSSIAN_SHARE, MIXED_TYPING_CHUNK_JOBS, MIXED_TYPING_DEFAULT_WORKERS,
)
from keyswitch.constants.training import TEST_LEDGER_LOCK_FILE_MODE  # noqa: E402
from devchat_floors import BASELINE, CANDIDATE, MODES, PAIRS, RELEASED, floors  # noqa: E402

DEVELOPMENT: Final = "dev"
TEST: Final = "test"
PINNED: Final = {DEVELOPMENT: DEVCHAT_DEV_CONTENT_SHA256, TEST: DEVCHAT_TEST_CONTENT_SHA256}
MIXED: Final = "mixed"
RUSSIAN: Final = "russian"
# The test's later batches, by name: the files `<name>.messages.jsonl.gz` and `<name>.members.jsonl.gz` beside the first
# one, each with the SHA-256 of its messages' content and where they came from.
TEST_BATCHES: Final = ROOT / "tools/devchat_test_batches.json"
INSTALLED: Final = ROOT / "src/keyswitch/resources/models/context_policy_v1.json"
BASELINE_ARTIFACT: Final = ROOT / "model/context_v3/baseline-context-v1.json"
BENCH: Final = "mixed_typing: the field read, a layout chosen by hand respected, the early switch off; four typing modes"
# Typed text never leaves a worker: a row is (message, mode, typed right) or the name of what failed.
Row = tuple[int, str, bool | str]


class Refusal(Exception):
    """A read the protocol refuses; its message is fixed text, never a word of a message."""


@dataclass(frozen=True)
class Member:
    text: str
    question: str
    mix: str


def ledger_root() -> Path:
    from evaluate_context_action_sequences import LEDGER_ROOT
    return LEDGER_ROOT.parent / "devchat-test-ledger"


def content_checksum(path: Path) -> str:
    with gzip.open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_members(path: Path) -> list[Member]:
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        return [Member(str(row["text"]), hashlib.sha256(f"{DEVCHAT_NAMESPACE}:{row['question']}".encode()).hexdigest(),
                       str(row["mix"])) for row in map(json.loads, handle)]


def split_files(split: str, directory: Path) -> list[tuple[Path, Path, str]]:
    """A split's (messages, members, pinned content SHA-256) files: the development split is one file, the test its
    first batch and every later batch TEST_BATCHES names."""

    files = [(directory / f"devchat-{split}.messages.jsonl.gz", directory / f"devchat-{split}.members.jsonl.gz", PINNED[split])]
    if split == TEST:
        for batch in cast(list[dict[str, str]], json.loads(TEST_BATCHES.read_text("utf-8"))["batches"]):
            name = batch["name"]
            files.append((directory / f"{name}.messages.jsonl.gz", directory / f"{name}.members.jsonl.gz", batch["content_sha256"]))
    return files


def members_of(files: Sequence[tuple[Path, Path, str]]) -> list[Member]:
    """The members of every file, a text a batch repeats counted once (in the first batch that holds it)."""

    seen: set[str] = set()
    members: list[Member] = []
    for _messages, path, _pin in files:
        for member in load_members(path):
            if member.text not in seen:
                seen.add(member.text)
                members.append(member)
    return members


def read_anything(access: Mapping[str, object]) -> bool:
    """Whether an earlier claim drew any message: one that drew none read no test."""

    return bool(cast(Mapping[str, list[str]], access["test_membership"])["messages"])


def draw(members: Sequence[Member], count: int | None, share: float,
         taken: tuple[set[str], set[str]] = (set(), set())) -> list[Member]:
    """`count` members with Latin words (all of them for None) and `share` as many of Russian words alone, in the seed's
    order, none of a message or question an earlier read took."""

    ordered = sorted((hashlib.sha256(f"{DEVCHAT_SEED}:{member.text}".encode()).hexdigest(), member) for member in members
                     if member.text not in taken[0] and member.question not in taken[1])
    mixed = [member for _key, member in ordered if member.mix == MIXED][:count]
    russian = [member for _key, member in ordered if member.mix == RUSSIAN][:int(len(mixed) * share)]
    return mixed + russian


def membership(chosen: Sequence[Member]) -> dict[str, list[str]]:
    return {"messages": sorted({member.text for member in chosen}), "questions": sorted({member.question for member in chosen})}


def texts_of(paths: Sequence[Path], chosen: Sequence[Member]) -> list[str]:
    """The texts of the chosen members, in their order: read only after the claim."""

    wanted = {member.text for member in chosen}
    found: dict[str, str] = {}
    for path in paths:
        with gzip.open(path, "rt", encoding="utf-8") as handle:
            for row in map(json.loads, handle):
                key = hashlib.sha256(str(row["text"]).encode()).hexdigest()
                if key in wanted:
                    found[key] = str(row["text"])
    if found.keys() != wanted:
        raise Refusal("the messages file lacks members the draw chose")
    return [found[member.text] for member in chosen]


def tatoeba_passed(candidate: str) -> bool:
    """Whether the candidate's Tatoeba test was read and passed (the immutable access and outcome of its ledger)."""

    from evaluate_context_action_sequences import LEDGER_ROOT
    for access in sorted(LEDGER_ROOT.glob("*.access.json")):
        identity = cast(dict[str, object], json.loads(access.read_bytes()))
        outcome = access.with_name(access.name.replace(".access.json", ".outcome.json"))
        if identity.get("artifact_sha256") == candidate and outcome.is_file():
            report = cast(dict[str, object], json.loads(outcome.read_bytes()))
            if report.get("split") == TEST and report.get("promotion_passed") is True:
                return True
    return False


def same_models(recorded: Mapping[str, Mapping[str, str]], pairs: Mapping[str, Mapping[str, str]]) -> bool:
    """Whether two reads ran the same models: each pair's checksum and version, wherever its file lay. The paths alone
    differed when the development record named the candidate by an absolute path and the promotion by a relative one,
    and the test was refused for models that were the same."""

    def models(value: Mapping[str, Mapping[str, str]]) -> dict[str, dict[str, str]]:
        return {name: {key: item for key, item in pair.items() if key != "path"} for name, pair in value.items()}

    return models(recorded) == models(pairs)


def runtime(directory: Path) -> dict[str, str]:
    """What a read ran on: the runtime the sequence protocol pins, the bench, this tool, the builder and the messages."""

    from evaluate_context_action_sequences import runtime_provenance
    pins = dict(runtime_provenance())
    for name in ("tools/mixed_typing.py", "tools/evaluate_devchat_test.py", "tools/build_devchat_messages.py"):
        pins[name] = checksum(ROOT / name)
    pins[TEST_BATCHES.relative_to(ROOT).as_posix()] = checksum(TEST_BATCHES)
    for path in sorted(directory.glob("devchat-*.jsonl.gz")):
        pins["messages/" + path.name] = content_checksum(path)
    return pins


def safe_evaluate(job: tuple[int, str, str, int, str]) -> Row:
    """One message in one mode; what went wrong comes back as its kind alone, never with the text it held."""

    import mixed_typing
    index, _text, mode, _choice, _prefix = job
    try:
        row = mixed_typing._evaluate(job)
    except Exception as error:  # noqa: BLE001 - a worker's exception may quote the message
        return index, mode, type(error).__name__
    if row.get("skipped"):
        return index, mode, "skipped"
    return index, mode, bool(row["ok"])


def typed(model: Path, texts: Sequence[str], workers: int) -> list[Row]:
    import mixed_typing
    jobs = [(index, text, mode, mixed_typing._draw(f"{DEVCHAT_SEED}:choice:{text}"), "")
            for index, text in enumerate(texts) for mode in mixed_typing.TYPING_MODES]
    with ProcessPoolExecutor(workers, initializer=mixed_typing._init, initargs=(str(model),)) as executor:
        return list(executor.map(safe_evaluate, jobs, chunksize=MIXED_TYPING_CHUNK_JOBS))


def counts(rows: Sequence[Row], chosen: Sequence[Member]) -> dict[str, dict[str, int]]:
    """Messages typed and typed right by mode and by mode and mix; a message a pair could not type counts as an error
    (one the mode does not apply to, a Russian message forgetting a change it has none of, counts nowhere)."""

    result: dict[str, dict[str, int]] = {}
    for index, mode, outcome in rows:
        if outcome == "skipped":
            continue
        for name in (mode, f"{mode}:{chosen[index].mix}"):
            entry = result.setdefault(name, {"n": 0, "ok": 0, "errors": 0})
            entry["n"] += 1
            entry["ok"] += int(outcome is True)
            entry["errors"] += int(isinstance(outcome, str))
    return dict(sorted(result.items()))


def discordance(candidate: Sequence[Row], released: Sequence[Row]) -> dict[str, dict[str, int]]:
    """By mode, the messages the candidate gets right and the released pair wrong (gains), and the reverse (losses)."""

    theirs = {(index, mode): outcome for index, mode, outcome in released}
    result = {mode: {"gains": 0, "losses": 0} for mode in MODES}
    for index, mode, outcome in candidate:
        other = theirs[index, mode]
        result[mode]["gains"] += int(outcome is True and other is not True)
        result[mode]["losses"] += int(outcome is not True and other is True)
    return result


def private_record(path: Path, value: object, *, exclusive: bool) -> None:
    """A file only its owner reads; `exclusive` refuses to replace one."""

    path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_CREAT | os.O_WRONLY | (os.O_EXCL if exclusive else os.O_TRUNC)
    with os.fdopen(os.open(path, flags, TEST_LEDGER_LOCK_FILE_MODE), "w", encoding="utf-8") as handle:
        handle.write(json.dumps(value, ensure_ascii=False, indent=1, sort_keys=True) + "\n")


def claim(identity: dict[str, object], root: Path) -> str:
    """Record a read of the test before any of its text is opened; refuse one that overlaps an earlier read or repeats
    a candidate's."""

    own = cast(dict[str, list[str]], identity["test_membership"])
    key = hashlib.sha256(json.dumps(own, sort_keys=True).encode()).hexdigest()
    candidate = cast(dict[str, dict[str, str]], identity["pairs"])[CANDIDATE]["sha256"]
    root.mkdir(parents=True, exist_ok=True)
    lock = root / ".access-lock"
    try:
        os.close(os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, TEST_LEDGER_LOCK_FILE_MODE))
    except FileExistsError:
        raise Refusal("the dev-chat test ledger is locked; no test was read") from None
    try:
        for path in sorted(root.glob("*.access.json")):
            prior = cast(dict[str, object], json.loads(path.read_bytes()))
            if cast(dict[str, dict[str, str]], prior["pairs"])[CANDIDATE]["sha256"] == candidate and read_anything(prior):
                raise Refusal("this candidate read the dev-chat test already")
            taken = cast(dict[str, list[str]], prior["test_membership"])
            if any(set(own[field]) & set(taken[field]) for field in ("messages", "questions")):
                raise Refusal("the draw overlaps messages or questions an earlier read took")
        private_record(root / (key + ".access.json"), identity, exclusive=True)
    finally:
        lock.unlink()
    return key


def consumed(root: Path) -> tuple[set[str], set[str]]:
    messages: set[str] = set()
    questions: set[str] = set()
    for path in sorted(root.glob("*.access.json")):
        taken = cast(dict[str, list[str]], json.loads(path.read_bytes())["test_membership"])
        messages.update(taken["messages"])
        questions.update(taken["questions"])
    return messages, questions


def evaluate(split: str, directory: Path, models: Mapping[str, Path], *, workers: int = MIXED_TYPING_DEFAULT_WORKERS,
             root: Path | None = None) -> dict[str, object]:
    ledger = root or ledger_root()
    files = split_files(split, directory)
    if any(content_checksum(messages) != pin for messages, _members, pin in files):
        raise Refusal(f"the {split} messages differ from the pinned ones")
    pairs = {name: {"path": str(path), "sha256": checksum(path), "model_version": str(json.loads(path.read_bytes())["version"])}
             for name, path in models.items()}
    candidate = pairs[CANDIDATE]["sha256"]
    pins = runtime(directory)
    development = ledger / f"{candidate}.dev.json"
    if split == TEST:
        for path in sorted(ledger.glob("*.access.json")):
            prior = cast(dict[str, object], json.loads(path.read_bytes()))
            if cast(dict[str, dict[str, str]], prior["pairs"])[CANDIDATE]["sha256"] == candidate and read_anything(prior):
                recorded_result = path.with_name(path.name.replace(".access.json", ".result.json"))
                if recorded_result.is_file():
                    return cast(dict[str, object], json.loads(recorded_result.read_bytes()))
                raise Refusal("this candidate read the dev-chat test already and no result was recorded")
        if not development.is_file():
            raise Refusal("no development result of this candidate is recorded; the test is not read")
        recorded = cast(dict[str, object], json.loads(development.read_bytes()))
        if not all(cast(dict[str, bool], recorded["floors"]).values()):
            raise Refusal("the candidate's development result holds no gain; the test is not read")
        if recorded["runtime"] != pins or not same_models(cast(Mapping[str, Mapping[str, str]], recorded["pairs"]), pairs):
            raise Refusal("the runtime or the pairs differ from the candidate's development result")
        if not tatoeba_passed(candidate):
            raise Refusal("the candidate passed no Tatoeba test; the dev-chat test is not read")
    chosen = draw(members_of(files), DEVCHAT_TEST_COUNT if split == TEST else None, MIXED_EVALUATE_RUSSIAN_SHARE,
                  consumed(ledger) if split == TEST else (set(), set()))
    if split == TEST and sum(member.mix == MIXED for member in chosen) < DEVCHAT_TEST_COUNT:
        raise Refusal("the test holds fewer fresh messages than a read takes; a new batch is needed and no test was read")
    batches = [pin for _messages, _members, pin in files]
    identity: dict[str, object] = {"protocol": "devchat-messages-v1", "split": split,
                                   "messages_sha256": batches[0] if len(batches) == 1 else batches,
                                   "seed": DEVCHAT_SEED, "russian_share": MIXED_EVALUATE_RUSSIAN_SHARE, "pairs": pairs,
                                   "bench": BENCH, "test_membership": membership(chosen)}
    key = claim(identity, ledger) if split == TEST else candidate
    result: dict[str, object] = {"split": split, "access": key, "messages": len(chosen), "pairs": pairs, "bench": BENCH}
    try:
        texts = texts_of([messages for messages, _members, _pin in files], chosen)
        rows = {name: typed(path, texts, workers) for name, path in models.items()}
        tables = {name: counts(rows[name], chosen) for name in PAIRS}
        paired = discordance(rows[CANDIDATE], rows[RELEASED])
        result.update({"results": tables, "discordance": paired, "floors": floors(tables, paired)})
    except Exception as error:  # noqa: BLE001 - every read is recorded, and its failure by kind alone
        result.update({"error": type(error).__name__, "floors": {"every_message_typed": False}})
    if split == DEVELOPMENT:
        result["runtime"] = pins
        private_record(development, result, exclusive=False)
    else:
        private_record(ledger / f"{key}.result.json", result, exclusive=True)
    return result


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--split", choices=(DEVELOPMENT, TEST), required=True)
    parser.add_argument("--directory", type=Path, required=True, help="the builder's output (tools/build_devchat_messages.py)")
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--released", type=Path, default=INSTALLED, help="the released pair's context model (read before install)")
    parser.add_argument("--baseline", type=Path, default=BASELINE_ARTIFACT)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=MIXED_TYPING_DEFAULT_WORKERS)
    arguments = parser.parse_args(argv)
    models = {CANDIDATE: arguments.candidate, RELEASED: arguments.released, BASELINE: arguments.baseline}
    try:
        result = evaluate(arguments.split, arguments.directory, models, workers=arguments.workers)
    except Refusal as refusal:
        print(f"devchat test not read: {refusal}", flush=True)
        return 1
    except Exception as error:  # noqa: BLE001 - nothing of a message is printed, whatever went wrong
        print(f"devchat test failed: {type(error).__name__}", flush=True)
        return 1
    private_record(arguments.output, result, exclusive=False)
    held = cast(dict[str, bool], result["floors"])
    tables = cast(dict[str, dict[str, dict[str, int]]], result.get("results", {}))
    summary = {name: {mode: tables[name][mode] for mode in MODES if mode in tables[name]} for name in tables}
    print(json.dumps({"split": arguments.split, "messages": result["messages"], "summary": summary,
                      "discordance": result.get("discordance"), "floors": held}), flush=True)
    return 0 if held and all(held.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
