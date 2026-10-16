#!/usr/bin/env python3
"""Promote a sealed context-v3 + prefix-v2 pair: checks first, the sealed test once, then the install.

A sealed test is read once. Everything the candidate's seal pins - the trainer, the runtime, the
tests that describe them - has to be final before that read: a file the seal pins changed after it
means a new seal, and a new seal never reads the same test again. On 06.10.2026 test v32 was read,
passed, and was lost that way: strict typing then rejected a test the seal pins, and the pair had to
be fitted again on a fresh corpus. This tool runs every check of the pinned files before the read
and refuses to touch the test until they hold:

1. the tracked tree is committed (the seal's provenance is the committed tree);
2. the candidate and its seal are valid for the corpus (evaluate_context_action_sequences);
3. strict typing (tools/typecheck.sh) and the named values (tools/check_named_values.py) pass;
4. every test module the seal pins passes, with the installed pair and then with the candidate pair
   in its place (a scratch worktree of the committed tree): the install must not break them either,
   and once the test is read a broken test module can only be fixed with a new pair;
5. with the candidate pair in that tree, no word of the owner's own typing that the last release got right
   comes out wrong (tools/field_regression_gate.py, the owner's decision of 15.10.2026): a pair that breaks
   one would be refused by the release anyway, and its sealed test would be spent for nothing;
6. the candidate's development result on the developer-chat messages is recorded and holds both floors
   (tools/evaluate_devchat_test.py --split dev): the developer-chat test is read only for such a candidate;
7. on the corpus's calibration frames the candidate nets, in every profile, at least what the installed pair nets
   there (tools/compare_context_candidates.py): the quality ratchet measures the receipt against that very number
   (tests/test_quality_ratchet.py), and candidate v63e, which passed both sealed tests, could not be installed for
   netting 23 frames less.

Then it reads the sealed test (once: an existing report is reused) and, once that passed and the candidate holds the
release receipt's restoration floors there (more rows typed in the wrong layout restored than the frozen baseline pair,
and the share of them; tests/test_quality_ratchet.py) - the corpus v64 pair passed the test's gates restoring exactly
what the baseline pair did and was installed, and the quality ratchet refused it - the sealed developer-chat test
(once, before the install: the released pair it is compared with is the installed one),
installs the pair, exports and verifies the release receipt, refreshes the boundary and prefix engine replays
and runs the public context and prefix gates - the order docs/model-training-cloud.md describes.

    PYTHONPATH=src:tools python3 tools/promote_context_action_pair.py \\
        --candidate <dir> --prefix-candidate <dir> --corpus <fit> --report <dir>/sequences-test.json
    ... --check-only    # the checks alone, no test bytes read
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Callable, Iterator, Sequence
from contextlib import AbstractContextManager, contextmanager
from pathlib import Path
from typing import Final, Protocol

ROOT: Final = Path(__file__).resolve().parents[1]
CONTEXT_ARTIFACT: Final = "context-action.json"
CONTEXT_SEAL: Final = "candidate-seal.json"
PREFIX_ARTIFACT: Final = "prefix-candidate.json"
PREFIX_SEAL: Final = "seal.json"
INSTALLED: Final = {
    CONTEXT_ARTIFACT: "src/keyswitch/resources/models/context_policy_v1.json",
    CONTEXT_SEAL: "model/context_v3/candidate-seal.json",
    PREFIX_ARTIFACT: "src/keyswitch/resources/models/prefix_policy_v1.json",
    PREFIX_SEAL: "model/prefix_v2/candidate-seal.json",
}
RECEIPT: Final = "model/context_v3/release-receipt.json"
# Where the comparison of the candidate's calibration with the installed pair's goes, in the candidate's directory.
CALIBRATION_COMPARISON: Final = "calibration-comparison.json"
TEST_MODULE_PREFIX: Final = "tests/test_"
# The developer-chat messages (tools/build_devchat_messages.py) are private; KEYSWITCH_DEVCHAT_DIRECTORY names another place.
DEVCHAT_DIRECTORY: Final = ROOT / ".t/devchat/messages"
DEVCHAT_DIRECTORY_VARIABLE: Final = "KEYSWITCH_DEVCHAT_DIRECTORY"

# Relative to the directory a command runs in: the sources of that tree, never those of another.
PYTHONPATH: Final = "src:tools"


class Runner(Protocol):
    def __call__(self, command: Sequence[str], cwd: Path = ROOT) -> int: ...


def run(command: Sequence[str], cwd: Path = ROOT) -> int:
    """A step's command in a tree's root (the repository's by default), its output passed through."""
    print("$ " + " ".join(command), flush=True)
    return subprocess.run(list(command), cwd=cwd, check=False, env={**os.environ, "PYTHONPATH": PYTHONPATH}).returncode


def installed_files(candidate: Path, prefix_candidate: Path) -> list[tuple[Path, str]]:
    """Each file of the pair and where the install puts it, relative to the tree's root."""
    return [(candidate / CONTEXT_ARTIFACT, INSTALLED[CONTEXT_ARTIFACT]), (candidate / CONTEXT_SEAL, INSTALLED[CONTEXT_SEAL]),
            (prefix_candidate / PREFIX_ARTIFACT, INSTALLED[PREFIX_ARTIFACT]),
            (prefix_candidate / PREFIX_SEAL, INSTALLED[PREFIX_SEAL])]


@contextmanager
def candidate_tree(candidate: Path, prefix_candidate: Path, root: Path = ROOT) -> Iterator[Path]:
    """A scratch worktree of the committed tree with the candidate pair installed, removed afterwards."""
    with tempfile.TemporaryDirectory(prefix="keyswitch-candidate-") as scratch:
        tree = Path(scratch) / "tree"
        subprocess.run(["git", "worktree", "add", "--detach", str(tree), "HEAD"], cwd=root, check=True, capture_output=True)
        try:
            for source, target in installed_files(candidate, prefix_candidate):
                shutil.copyfile(source, tree / target)
            yield tree
        finally:
            subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=root, check=False, capture_output=True)


def pinned_test_modules(seal: dict[str, object]) -> list[str]:
    """The test modules the seal's provenance names, as unittest module names."""
    provenance = seal.get("provenance")
    if not isinstance(provenance, dict):
        raise ValueError("the seal names no provenance")
    return sorted(Path(name).stem for name in provenance if isinstance(name, str)
                  and name.startswith(TEST_MODULE_PREFIX) and name.endswith(".py"))


def uncommitted(root: Path = ROOT) -> list[str]:
    """Tracked files that differ from the commit: the seal's provenance is the committed tree."""
    output = subprocess.run(["git", "diff", "--name-only", "HEAD"], cwd=root, check=True, capture_output=True, text=True).stdout
    return [line for line in output.splitlines() if line.strip()]


def preflight(candidate: Path, prefix_candidate: Path, corpus: Path, runner: Runner = run,
              changed: Callable[[], list[str]] = uncommitted,
              tree: Callable[[Path, Path], AbstractContextManager[Path]] = candidate_tree) -> list[str]:
    """Every check of the pinned files, before any test byte is read; the failures, empty if none."""
    failures: list[str] = []
    if pending := changed():
        failures.append("uncommitted tracked files: " + ", ".join(pending))
    import evaluate_context_action_sequences as evaluator

    seal_path = candidate / CONTEXT_SEAL
    try:
        evaluator.validate_candidate_seal(candidate / CONTEXT_ARTIFACT, seal_path, corpus)
    except (OSError, ValueError) as error:
        failures.append(f"candidate seal: {error}")
    for path in (prefix_candidate / PREFIX_ARTIFACT, prefix_candidate / PREFIX_SEAL):
        if not path.is_file():
            failures.append(f"missing {path}")
    python = sys.executable
    checks = [("strict typing", ["bash", "tools/typecheck.sh"]),
              ("named values", [python, "tools/check_named_values.py"])]
    tests: list[str] = []
    if seal_path.is_file():
        modules = pinned_test_modules(json.loads(seal_path.read_bytes()))
        # Each module in a process of its own, as CI runs them (tools/run_test_modules.py).
        tests = [python, "tools/run_test_modules.py", *(module + ".py" for module in modules),
                 "--", python, "-m", "unittest", "discover", "-s", "tests", "-p"]
        checks.append(("tests the seal pins", tests))
    for name, command in checks:
        if runner(command) != 0:
            failures.append(name)
    # The same modules with the candidate pair installed, once everything else holds: the files to
    # install exist and the tree is the commit. Then the owner's typing in that tree against the last release.
    if tests and not failures:
        with tree(candidate, prefix_candidate) as root:
            if runner(tests, root) != 0:
                failures.append("tests the seal pins, with the candidate pair installed")
            if runner([python, "tools/field_regression_gate.py", "--candidate-root", str(root)]) != 0:
                failures.append("the owner's typing: a word the last release got right comes out wrong, or the field "
                                "regression gate could not run (tools/field_regression_gate.py)")
    if not development_gain(candidate / CONTEXT_ARTIFACT):
        failures.append("no developer-chat development result of the candidate holds both floors "
                        "(tools/evaluate_devchat_test.py --split dev)")
    if not failures:
        failures.extend(f"the calibration nets less than the installed pair's in the {name} profile "
                        "(tools/compare_context_candidates.py): the quality ratchet would refuse the install"
                        for name in calibration_shortfall(candidate, corpus, runner))
    return failures


def calibration_shortfall(candidate: Path, corpus: Path, runner: Runner = run) -> list[str]:
    """The profiles in which the candidate nets less on the corpus's calibration frames than the installed pair, as
    tools/compare_context_candidates.py scores both (every profile, when the comparison could not run)."""
    output = candidate / CALIBRATION_COMPARISON
    output.unlink(missing_ok=True)
    command = [sys.executable, "tools/compare_context_candidates.py", "--corpus", str(corpus),
               "--artifact", f"installed={ROOT / INSTALLED[CONTEXT_ARTIFACT]}",
               "--artifact", f"candidate={candidate / CONTEXT_ARTIFACT}", "--output", str(output)]
    if runner(command) != 0 or not output.is_file():
        return ["every"]
    profiles = json.loads(output.read_bytes())["calibration"]
    return sorted(name for name, pairs in profiles.items()
                  if pairs["candidate"]["by_class"]["all"]["net"] < pairs["installed"]["by_class"]["all"]["net"])


def development_gain(artifact: Path) -> bool:
    """Whether the candidate's developer-chat development result is recorded and holds every floor."""
    import evaluate_devchat_test as devchat
    try:
        record = json.loads((devchat.ledger_root() / f"{devchat.checksum(artifact)}.dev.json").read_bytes())
    except (OSError, ValueError):
        return False
    floors = record.get("floors")
    return isinstance(floors, dict) and bool(floors) and all(value is True for value in floors.values())


def promote(candidate: Path, prefix_candidate: Path, corpus: Path, report: Path, runner: Runner = run) -> int:
    """The sealed test (once), the install, the receipt, the engine replays and the gates."""
    python = sys.executable
    if not report.exists():
        status = runner([python, "tools/evaluate_context_action_sequences.py",
                         "--candidate", str(candidate / CONTEXT_ARTIFACT), "--seal", str(candidate / CONTEXT_SEAL),
                         "--prefix-candidate", str(prefix_candidate / PREFIX_ARTIFACT),
                         "--prefix-seal", str(prefix_candidate / PREFIX_SEAL),
                         "--corpus", str(corpus), "--split", "test", "--output", str(report)])
        if status != 0:
            print("the sealed test did not pass; nothing is installed", flush=True)
            return status
    if shortfall := restoration_shortfall(report):
        print("the sealed test leaves the candidate short of the release receipt's restoration floors ("
              + "; ".join(shortfall) + "); nothing is installed and the developer-chat test is not read", flush=True)
        return 1
    # The developer-chat test, while the released pair is still the installed one it is compared with.
    devchat = Path(os.environ.get(DEVCHAT_DIRECTORY_VARIABLE, DEVCHAT_DIRECTORY))
    status = runner([python, "tools/evaluate_devchat_test.py", "--split", "test", "--directory", str(devchat),
                     "--candidate", str(candidate / CONTEXT_ARTIFACT), "--released", INSTALLED[CONTEXT_ARTIFACT],
                     "--output", str(report.with_name("devchat-test.json"))])
    if status != 0:
        print("the sealed developer-chat test did not pass; nothing is installed", flush=True)
        return status
    for source, target in installed_files(candidate, prefix_candidate):
        shutil.copyfile(source, ROOT / target)
    # The receipt is an immutable record: the one of the replaced pair goes first.
    (ROOT / RECEIPT).unlink(missing_ok=True)
    steps = [
        [python, "tools/verify_context_action_model.py", "--export", "--artifact", INSTALLED[CONTEXT_ARTIFACT],
         "--prefix-artifact", INSTALLED[PREFIX_ARTIFACT], "--seal", INSTALLED[CONTEXT_SEAL],
         "--prefix-seal", INSTALLED[PREFIX_SEAL], "--report", str(report), "--corpus", str(corpus), "--output", RECEIPT],
        [python, "tools/verify_context_action_model.py", "--verify"],
        [python, "tools/evaluate_boundary_engine.py", "--refresh-runtime"],
        [python, "tools/evaluate_prefix_engine.py", "--refresh-runtime"],
        [python, "tools/verify_context_model.py", "--replay"],
        [python, "tools/verify_prefix_model.py"],
    ]
    for command in steps:
        if (status := runner(command)) != 0:
            return status
    return 0


def restoration_shortfall(report: Path) -> list[str]:
    """The profiles and settings modes in which the sealed test leaves the candidate short of the release receipt's
    floors (tests/test_quality_ratchet.py): restoring more rows typed in the wrong layout than the frozen baseline pair by
    RECEIPT_MIN_RESTORATION_MARGIN, and at least RECEIPT_MIN_RESTORED_SHARE of them."""
    sys.path.insert(0, str(ROOT / "tests"))
    from fixture_values.quality_floors import RECEIPT_MIN_RESTORATION_MARGIN, RECEIPT_MIN_RESTORED_SHARE

    failures: list[str] = []
    for name, profile in sorted(json.loads(report.read_bytes())["profiles"].items()):
        for mode, counts in (("default", profile["counts"]), ("early_off", profile["early_off"]["counts"])):
            candidate, baseline = counts["candidate"], counts["baseline"]
            if candidate["exactly_restored"] - baseline["exactly_restored"] < RECEIPT_MIN_RESTORATION_MARGIN[mode]:
                failures.append(f"{name} {mode}: {candidate['exactly_restored']} restored against the baseline pair's "
                                f"{baseline['exactly_restored']}")
            share = RECEIPT_MIN_RESTORED_SHARE.get(mode)
            if share is not None and candidate["exactly_restored"] < share * candidate["initially_wrong"]:
                failures.append(f"{name} {mode}: {candidate['exactly_restored']} of {candidate['initially_wrong']} restored")
    return failures


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--candidate", type=Path, required=True, help="the context candidate's directory")
    parser.add_argument("--prefix-candidate", type=Path, required=True, help="the prefix candidate's directory")
    parser.add_argument("--corpus", type=Path, required=True, help="the frozen fitting corpus")
    parser.add_argument("--report", type=Path, required=True, help="where the sealed test report goes")
    parser.add_argument("--check-only", action="store_true", help="run the checks and stop before the test")
    arguments = parser.parse_args(argv)
    # The seal is checked in this process: against this tree's runtime, not an installed package's.
    sys.path.insert(0, str(ROOT / "src"))
    failures = preflight(arguments.candidate, arguments.prefix_candidate, arguments.corpus)
    if failures:
        print("not reading the sealed test; fix first: " + "; ".join(failures), flush=True)
        return 1
    print("every check of the pinned files holds", flush=True)
    if arguments.check_only:
        return 0
    return promote(arguments.candidate, arguments.prefix_candidate, arguments.corpus, arguments.report)


if __name__ == "__main__":
    raise SystemExit(main())
