#!/usr/bin/env python3
"""Run test modules in parallel processes: each module is one run of the command given after `--`.

The suites under coverage ran one module after another in one process: 14.7 minutes on the Linux
runner, 13.5 on the Windows Arm one. Each module now runs on its own, as many at a time as the
runner has cores, so a coverage command must write parallel data files (`coverage run
--parallel-mode`) that `coverage combine` joins before the report. Modules named by --serial share
one display or desktop (GTK windows, the clipboard, AT-SPI): they run one after another in a
single lane, beside the others. The largest modules start first; each module's output is printed
whole when it ends, and the run fails when any module fails.

    python3 tools/run_test_modules.py --serial 'test_ui.py' 'test_*.py' -- \\
        python3 -m coverage run --parallel-mode -m unittest discover -s tests -v -p

The module's file name is appended to the command (the value of `discover -p`).

`--part K/N` runs the K-th of N parts of the chosen modules, so that N runners share one suite: the
modules are dealt from the largest down in snake order (1..N, N..1, ...), a file's size standing in
for its running time, so that each part gets a like share of the long ones. The parts of one tree
are disjoint and together hold every chosen module.
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import subprocess
import sys
import threading
import time
from collections.abc import Sequence
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Final

ROOT: Final = Path(__file__).resolve().parents[1]
TESTS: Final = ROOT / "tests"
JOBS_VARIABLE: Final = "KEYSWITCH_TEST_JOBS"


def modules(patterns: Sequence[str], directory: Path = TESTS) -> list[str]:
    """The test files the patterns name, each once; a pattern that names none is an error."""

    names = sorted(path.name for path in directory.glob("test_*.py"))
    chosen: list[str] = []
    for pattern in patterns:
        matched = fnmatch.filter(names, pattern)
        if not matched:
            raise ValueError(f"no test module matches {pattern}")
        chosen.extend(name for name in matched if name not in chosen)
    return chosen


def lanes(names: Sequence[str], serial: Sequence[str], directory: Path = TESTS) -> list[list[str]]:
    """The modules in the order they start: the serial lane first, then the others, largest first."""

    shared = [name for name in names if any(fnmatch.fnmatch(name, pattern) for pattern in serial)]
    alone = sorted((name for name in names if name not in shared),
                   key=lambda name: (-(directory / name).stat().st_size, name))
    return ([shared] if shared else []) + [[name] for name in alone]


def part(names: Sequence[str], index: int, count: int, directory: Path = TESTS) -> list[str]:
    """The modules of part `index` (from 1) of `count`: dealt from the largest down in snake order."""

    ordered = sorted(names, key=lambda name: (-(directory / name).stat().st_size, name))
    rounds = count + count
    return [name for position, name in enumerate(ordered)
            if min(position % rounds, rounds - 1 - position % rounds) == index - 1]


def parse_part(text: str) -> tuple[int, int]:
    """`K/N`: part K of N, 1 <= K <= N."""

    index, separator, count = text.partition("/")
    if not (separator and index.isdigit() and count.isdigit() and 1 <= int(index) <= int(count)):
        raise argparse.ArgumentTypeError(f"a part is K/N with 1 <= K <= N, not {text!r}")
    return int(index), int(count)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--jobs", type=int, default=int(os.environ.get(JOBS_VARIABLE, os.cpu_count() or 1)),
                        help=f"modules at a time (default: {JOBS_VARIABLE} or the number of cores)")
    parser.add_argument("--serial", action="append", default=[], metavar="PATTERN",
                        help="modules that run one after another in one lane")
    parser.add_argument("--part", type=parse_part, default=(1, 1), metavar="K/N",
                        help="run only part K of N of the modules (default: all of them)")
    parser.add_argument("patterns", nargs="+", metavar="PATTERN", help="test files in tests/, as glob patterns")
    given = list(sys.argv[1:] if argv is None else argv)
    if "--" not in given or given.index("--") == len(given) - 1:
        parser.error("give the command to run each module with after --")
    split = given.index("--")
    arguments, command = parser.parse_args(given[:split]), given[split + 1:]
    index, count = arguments.part
    names = part(modules(arguments.patterns, TESTS), index, count, TESTS)
    order = lanes(names, arguments.serial, TESTS)
    lock = threading.Lock()
    failed: list[str] = []

    def run(lane: list[str]) -> None:
        for name in lane:
            started = time.monotonic()
            result = subprocess.run([*command, name], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    check=False)
            elapsed = time.monotonic() - started
            with lock:
                status = "passed" if result.returncode == 0 else f"FAILED with exit code {result.returncode}"
                print(f"==== {name}: {status} in {elapsed:.1f} s", flush=True)
                sys.stdout.buffer.write(result.stdout)
                sys.stdout.flush()
                if result.returncode:
                    failed.append(name)

    started = time.monotonic()
    with ThreadPoolExecutor(max_workers=max(1, arguments.jobs)) as pool:
        for future in [pool.submit(run, lane) for lane in order]:
            future.result()
    print(f"==== {len(names)} modules in {time.monotonic() - started:.1f} s, {arguments.jobs} at a time; "
          + (f"failed: {', '.join(sorted(failed))}" if failed else "all passed"), flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
