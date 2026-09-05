#!/usr/bin/env python3
"""One command that turns a prepared working tree into a published release.

The mechanical half of a KeySwitch release is always the same: propagate the
version, close the changelog section, run the selected Linux verification profile,
commit, tag, push and wait for the packages to appear in the GitHub release.
This script performs all of it in order and stops at the first step that does
not hold, printing what is wrong and what to do about it.

What it does not do is write prose: the entries under ``## Unreleased`` in
``CHANGELOG.md`` and the text of ``RELEASE_NOTES.md`` are the author's, and the
script refuses to run without them.

Typical use::

    python3 tools/release.py --version X.Y.Z --dry-run
    python3 tools/release.py --version X.Y.Z

Replace X.Y.Z with an unused version. Publication stages every working-tree
change with git add -A. The dry run checks preparation without writing files
or running verification. A retry normally runs verification again; an existing
tag is allowed only at the same clean HEAD. It does not roll back a pushed tag
or restart a failed Actions run. See docs/verification.md for recovery.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import shutil
import subprocess
import sys
import time
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final

sys.path.insert(0, str(Path(__file__).resolve().parent))

import release_pipeline as pipeline  # noqa: E402


PROJECT_ROOT: Final[Path] = pipeline.PROJECT_ROOT
VERSION_PATTERN: Final[re.Pattern[str]] = re.compile(r"^\d+\.\d+\.\d+$")
CHANGELOG: Final[Path] = PROJECT_ROOT / "CHANGELOG.md"
RELEASE_NOTES: Final[Path] = PROJECT_ROOT / "RELEASE_NOTES.md"
UNRELEASED_HEADING: Final[str] = "## Unreleased"
# Assets the release workflow attaches; a release without one of them is a
# failed publication even when the workflow reports success.
RELEASE_ASSETS: Final[tuple[str, ...]] = (
    "keyswitch_{version}_amd64.deb",
    "KeySwitch-Setup-{version}-x64.exe",
    "KeySwitch-{version}-windows-x64.zip",
    "SHA256SUMS",
)
CI_APPEARANCE_TIMEOUT: Final[float] = 180.0
CI_POLL_SECONDS: Final[float] = 5.0


class ReleaseError(Exception):
    """A step failed; the message tells the operator what to fix."""


@dataclass(frozen=True)
class VersionSite:
    """One file that spells the version out, and how it spells it."""

    path: Path
    pattern: str
    replacement: str

    def apply(self, previous: str, version: str, *, write: bool = True) -> int:
        text = self.path.read_text(encoding="utf-8")
        search = self.pattern.format(version=re.escape(previous))
        updated, count = re.subn(
            search, self.replacement.format(version=version), text, flags=re.MULTILINE
        )
        if count == 0:
            raise ReleaseError(
                f"{pipeline.relative(self.path)} does not mention version "
                f"{previous} in the expected form ({self.pattern})"
            )
        if write and updated != text:
            self.path.write_text(updated, encoding="utf-8")
        return count


def version_sites() -> tuple[VersionSite, ...]:
    return (
        VersionSite(
            PROJECT_ROOT / "pyproject.toml",
            r'^version = "{version}"$',
            'version = "{version}"',
        ),
        VersionSite(
            PROJECT_ROOT / "src" / "keyswitch" / "__init__.py",
            r'^__version__ = "{version}"$',
            '__version__ = "{version}"',
        ),
        VersionSite(
            PROJECT_ROOT / "packaging" / "keyswitch.1",
            r'"KeySwitch {version}"',
            '"KeySwitch {version}"',
        ),
        VersionSite(
            PROJECT_ROOT / "tests" / "test_detector_engine_branches.py",
            r'"keyswitch_version"\], "{version}"',
            '"keyswitch_version"], "{version}"',
        ),
        VersionSite(PROJECT_ROOT / "README.md", r"{version}", "{version}"),
        VersionSite(PROJECT_ROOT / "README.en.md", r"{version}", "{version}"),
    )


@dataclass(frozen=True)
class Options:
    version: str
    branch: str
    remote: str
    profile: str
    message_file: Path | None
    skip_pipeline: bool
    skip_ci: bool
    dry_run: bool
    ci_timeout: float


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------


def announce(step: int, total: int, message: str) -> None:
    print(f"==> [{step}/{total}] {message}", flush=True)


def note(message: str) -> None:
    print(f"    {message}", flush=True)


# --------------------------------------------------------------------------
# Git helpers
# --------------------------------------------------------------------------


def git(*arguments: str) -> str:
    """Run git and fail loudly; git_output() is the quiet variant."""

    completed = subprocess.run(
        ["git", *arguments],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise ReleaseError(f"git {' '.join(arguments)} failed: {detail}")
    return completed.stdout.strip()


def dirty_paths() -> list[str]:
    return [line for line in git("status", "--porcelain").splitlines() if line.strip()]


def tag_exists_locally(tag: str) -> bool:
    return bool(pipeline.git_output("rev-parse", "--verify", "--quiet", f"{tag}^{{commit}}"))


def tag_exists_remotely(remote: str, tag: str) -> bool:
    return bool(pipeline.git_output("ls-remote", "--tags", remote, tag))


# --------------------------------------------------------------------------
# Steps
# --------------------------------------------------------------------------


def check_preconditions(options: Options) -> None:
    if VERSION_PATTERN.fullmatch(options.version) is None:
        raise ReleaseError(f"version {options.version!r} is not MAJOR.MINOR.PATCH")
    if not (PROJECT_ROOT / ".git").exists():
        raise ReleaseError(f"{PROJECT_ROOT} is not a git working tree")
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    if branch != options.branch:
        raise ReleaseError(
            f"HEAD is on {branch!r}, not {options.branch!r}; pass --branch to release "
            "from this branch on purpose"
        )
    if not options.skip_ci and not command_exists("gh"):
        raise ReleaseError(
            "the GitHub CLI (gh) is required to watch the release workflow; "
            "install it, or pass --skip-ci to stop after the push"
        )
    tag = f"v{options.version}"
    if tag_exists_locally(tag):
        head = git("rev-parse", "HEAD")
        tagged = git("rev-parse", f"{tag}^{{commit}}")
        if tagged != head or dirty_paths():
            raise ReleaseError(
                f"{tag} already exists at {tagged[:12]} while the tree has moved on "
                f"(HEAD {head[:12]}, {len(dirty_paths())} dirty paths); bump the version"
            )
    note(f"branch {branch}, {len(dirty_paths())} paths to release")


def command_exists(name: str) -> bool:
    return shutil.which(name) is not None


def load_json_text(payload: str, label: str) -> object:
    try:
        return json.loads(payload)
    except json.JSONDecodeError as error:
        raise ReleaseError(f"{label} did not return valid JSON: {error}") from error


def apply_version(options: Options) -> None:
    previous = pipeline.project_version()
    if previous == options.version:
        note(f"every file already declares {options.version}")
        verify_version_consistency(options.version)
        return
    for site in version_sites():
        count = site.apply(previous, options.version, write=not options.dry_run)
        verb = "would rewrite" if options.dry_run else "rewrote"
        note(f"{verb} {count} occurrence(s) in {pipeline.relative(site.path)}")
    if not options.dry_run:
        verify_version_consistency(options.version)


def verify_version_consistency(version: str) -> None:
    """The pipeline checks this too; failing here saves a 30 minute run."""

    stale: list[str] = []
    for site in version_sites():
        text = site.path.read_text(encoding="utf-8")
        if re.search(site.pattern.format(version=re.escape(version)), text, re.MULTILINE) is None:
            stale.append(pipeline.relative(site.path))
    if stale:
        raise ReleaseError("these files do not declare " + version + ": " + ", ".join(stale))


def close_changelog(version: str, *, dry_run: bool = False) -> None:
    text = CHANGELOG.read_text(encoding="utf-8")
    sections = pipeline.changelog_sections(text)
    if any(heading.split(" ")[0] == version for heading in sections if heading != "Unreleased"):
        note(f"CHANGELOG.md already has a section for {version}")
        return
    if not sections.get("Unreleased"):
        raise ReleaseError(
            "CHANGELOG.md has neither a section for "
            f"{version} nor entries under {UNRELEASED_HEADING}; describe the "
            "release there first"
        )
    today = dt.date.today().isoformat()
    heading = f"## {version} — {today}"
    entries = len(sections["Unreleased"])
    if dry_run:
        note(f"would move {entries} Unreleased entries under {heading}")
        return
    # Entries parsed under "Unreleased" mean the heading line is present.
    updated = text.replace(
        UNRELEASED_HEADING + "\n",
        UNRELEASED_HEADING + "\n\n" + heading + "\n",
        1,
    )
    CHANGELOG.write_text(updated, encoding="utf-8")
    note(f"moved {entries} Unreleased entries under {heading}")


def check_release_notes(version: str) -> None:
    text = RELEASE_NOTES.read_text(encoding="utf-8")
    match = re.match(r"# KeySwitch (\S+)", text)
    if match is None or match.group(1) != version:
        found = "nothing" if match is None else match.group(1)
        raise ReleaseError(
            f"RELEASE_NOTES.md describes {found}, not {version}; write the notes for "
            "this release first"
        )
    missing = [
        asset.format(version=version)
        for asset in RELEASE_ASSETS
        if asset != "SHA256SUMS" and asset.format(version=version) not in text
    ]
    if missing:
        raise ReleaseError(
            "RELEASE_NOTES.md does not name the published files: " + ", ".join(missing)
        )
    note("release notes describe this version")


def run_pipeline(options: Options) -> Path:
    run_directory = pipeline.DEFAULT_PIPELINE_ROOT / pipeline.run_identifier(options.profile)
    command = [
        sys.executable,
        str(PROJECT_ROOT / "tools" / "release_pipeline.py"),
        "run",
        "--profile",
        options.profile,
        "--run-dir",
        str(run_directory),
    ]
    note(" ".join(command))
    completed = subprocess.run(command, cwd=PROJECT_ROOT, check=False)
    summary = run_directory / pipeline.SUMMARY_JSON
    if completed.returncode != 0:
        raise ReleaseError(
            f"the verification contour failed (exit {completed.returncode}); "
            f"read {pipeline.relative(run_directory / pipeline.SUMMARY_MARKDOWN)}"
            + failed_phase_summary(summary)
        )
    status = pipeline_status(summary)
    if status != "passed":
        raise ReleaseError(
            f"the verification contour reported {status!r}; read "
            f"{pipeline.relative(run_directory / pipeline.SUMMARY_MARKDOWN)}"
            + failed_phase_summary(summary)
        )
    note(f"verification passed: {pipeline.relative(run_directory)}")
    return run_directory


def pipeline_status(summary: Path) -> str:
    if not summary.is_file():
        raise ReleaseError(f"the verification contour wrote no summary at {summary}")
    payload = pipeline.load_json_object(summary, "pipeline summary")
    return pipeline.as_str(
        pipeline.lookup(payload, "pipeline", "status"), "pipeline.status"
    )


def failed_phase_summary(summary: Path) -> str:
    if not summary.is_file():
        return ""
    try:
        payload = pipeline.load_json_object(summary, "pipeline summary")
        phases = pipeline.lookup(payload, "phases")
    except pipeline.PhaseFailure:
        return ""
    if not isinstance(phases, list):
        return ""
    failed: list[str] = []
    for phase in phases:
        entry = phase if isinstance(phase, dict) else {}
        if entry.get("status") == "failed":
            failed.append(str(entry.get("name", "?")))
    if not failed:
        return ""
    return "; failed phases: " + ", ".join(failed)


def build_commit_message(options: Options, run_directory: Path | None) -> str:
    if options.message_file is not None:
        if not options.message_file.is_file():
            raise ReleaseError(f"commit message file is missing: {options.message_file}")
        return options.message_file.read_text(encoding="utf-8")
    sections = pipeline.changelog_sections(CHANGELOG.read_text(encoding="utf-8"))
    entries = next(
        (items for heading, items in sections.items() if heading.split(" ")[0] == options.version),
        [],
    )
    if not entries:
        raise ReleaseError(
            f"CHANGELOG.md has no entries under {options.version}; nothing to describe"
        )
    body = "\n".join(entries)
    verification = (
        "Verified by one tools/release_pipeline.py --profile "
        f"{options.profile} run"
        + (f" ({pipeline.relative(run_directory)})." if run_directory is not None else ".")
    )
    return f"Release KeySwitch {options.version}\n\n{body}\n\n{verification}\n"


def commit_release(options: Options, message: str) -> bool:
    if not dirty_paths():
        note("nothing to commit; the tree already holds the release")
        return False
    git("add", "-A")
    subject = message.splitlines()[0]
    completed = subprocess.run(
        ["git", "commit", "-F", "-"],
        cwd=PROJECT_ROOT,
        input=message,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise ReleaseError(f"git commit failed: {detail}")
    note(f"committed {git('rev-parse', '--short', 'HEAD')} {subject}")
    return True


def tag_release(version: str) -> str:
    tag = f"v{version}"
    if tag_exists_locally(tag):
        note(f"{tag} already points at {git('rev-parse', '--short', f'{tag}^{{commit}}')}")
        return tag
    git("tag", "-a", tag, "-m", f"KeySwitch {version}")
    note(f"created {tag}")
    return tag


def push_release(options: Options, tag: str) -> None:
    ahead = pipeline.git_output(
        "rev-list", "--count", f"{options.remote}/{options.branch}..{options.branch}"
    )
    if ahead in ("", "0") and tag_exists_remotely(options.remote, tag):
        note(f"{options.remote} already has {options.branch} and {tag}")
        return
    git("push", options.remote, options.branch, "--follow-tags")
    note(f"pushed {options.branch} and {tag} to {options.remote}")


def gh(*arguments: str, timeout: float | None = None) -> str:
    completed = subprocess.run(
        ["gh", *arguments],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
        timeout=timeout,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise ReleaseError(f"gh {' '.join(arguments)} failed: {detail}")
    return completed.stdout.strip()


def wait_for_workflow(options: Options, tag: str) -> None:
    identifier = workflow_run_id(tag)
    note(f"watching workflow run {identifier}")
    completed = subprocess.run(
        ["gh", "run", "watch", identifier, "--exit-status"],
        cwd=PROJECT_ROOT,
        check=False,
        timeout=options.ci_timeout,
    )
    if completed.returncode != 0:
        raise ReleaseError(
            f"the release workflow failed; inspect it with "
            f"`gh run view {identifier} --log-failed`"
        )
    note("release workflow succeeded")


def workflow_run_id(tag: str) -> str:
    deadline = time.monotonic() + CI_APPEARANCE_TIMEOUT
    while True:
        payload = gh(
            "run",
            "list",
            "--branch",
            tag,
            "--limit",
            "10",
            "--json",
            "databaseId,workflowName,event",
        )
        for entry in parse_run_list(payload):
            if entry.event == "push" and "release" in entry.workflow.casefold():
                return str(entry.identifier)
        if time.monotonic() >= deadline:
            raise ReleaseError(
                f"no release workflow run appeared for {tag} within "
                f"{int(CI_APPEARANCE_TIMEOUT)}s; check the Actions tab"
            )
        time.sleep(CI_POLL_SECONDS)


@dataclass(frozen=True)
class WorkflowRun:
    identifier: int
    workflow: str
    event: str


def parse_run_list(payload: str) -> tuple[WorkflowRun, ...]:
    document = load_json_text(payload, "gh run list")
    if not isinstance(document, list):
        raise ReleaseError("gh run list did not return a JSON array")
    runs: list[WorkflowRun] = []
    for item in document:
        entry = pipeline.as_object(item, "workflow run")
        runs.append(
            WorkflowRun(
                pipeline.as_int(entry.get("databaseId", 0), "databaseId"),
                pipeline.as_str(entry.get("workflowName", ""), "workflowName"),
                pipeline.as_str(entry.get("event", ""), "event"),
            )
        )
    return tuple(runs)


def verify_published_release(version: str, tag: str) -> str:
    payload = gh("release", "view", tag, "--json", "url,assets")
    document = load_json_text(payload, "gh release view")
    release = pipeline.as_object(document, "release")
    assets = release.get("assets")
    if not isinstance(assets, list):
        raise ReleaseError(f"the release {tag} lists no assets")
    published = {
        pipeline.as_str(pipeline.as_object(item, "asset").get("name", ""), "asset name")
        for item in assets
    }
    missing = [
        expected.format(version=version)
        for expected in RELEASE_ASSETS
        if expected.format(version=version) not in published
    ]
    if missing:
        raise ReleaseError(
            f"the release {tag} is missing: " + ", ".join(missing)
        )
    url = pipeline.as_str(release.get("url", ""), "release url")
    note(f"published assets: {', '.join(sorted(published))}")
    return url


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def release(options: Options) -> None:
    total = 4 if options.dry_run else (8 if not options.skip_ci else 6)
    announce(1, total, f"checking the working tree for KeySwitch {options.version}")
    check_preconditions(options)

    announce(2, total, "propagating the version")
    apply_version(options)

    announce(3, total, "closing the changelog section")
    close_changelog(options.version, dry_run=options.dry_run)

    announce(4, total, "checking the release notes")
    check_release_notes(options.version)

    if options.dry_run:
        note("dry run: nothing was written, stopping before the contour")
        return

    run_directory: Path | None = None
    if options.skip_pipeline:
        announce(5, total, "skipping the verification contour (--skip-pipeline)")
    else:
        announce(5, total, "running the verification contour")
        run_directory = run_pipeline(options)

    announce(6, total, "committing, tagging and pushing")
    commit_release(options, build_commit_message(options, run_directory))
    tag = tag_release(options.version)
    push_release(options, tag)

    if options.skip_ci:
        note("skipping the release workflow (--skip-ci)")
        return

    announce(7, total, "waiting for the release workflow")
    wait_for_workflow(options, tag)

    announce(8, total, "verifying the published release")
    url = verify_published_release(options.version, tag)
    print(f"\nKeySwitch {options.version} is released: {url}", flush=True)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="release.py",
        description=(
            "Release KeySwitch: propagate the version, close the changelog, run the "
            "verification contour, commit, tag, push and confirm the published packages."
        ),
    )
    parser.add_argument(
        "--version",
        help="version to release (default: the version in pyproject.toml)",
    )
    parser.add_argument("--branch", default="main")
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--profile", default="release", choices=sorted(pipeline.PROFILES))
    parser.add_argument(
        "--message-file",
        type=Path,
        help="file with the complete commit message (default: built from the changelog)",
    )
    parser.add_argument(
        "--skip-pipeline",
        action="store_true",
        help="skip local verification (does not validate or select an earlier run)",
    )
    parser.add_argument(
        "--skip-ci",
        action="store_true",
        help="stop after the push instead of waiting for the release workflow",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="check the tree, version, changelog and notes, then stop",
    )
    parser.add_argument("--ci-timeout", type=float, default=2400.0)
    return parser


def parse_arguments(argv: Sequence[str] | None) -> Options:
    arguments = build_parser().parse_args(argv)
    version = arguments.version or pipeline.project_version()
    timeout = float(arguments.ci_timeout)
    if timeout <= 0:
        raise ReleaseError("--ci-timeout must be positive")
    return Options(
        version=str(version),
        branch=str(arguments.branch),
        remote=str(arguments.remote),
        profile=str(arguments.profile),
        message_file=arguments.message_file,
        skip_pipeline=bool(arguments.skip_pipeline),
        skip_ci=bool(arguments.skip_ci),
        dry_run=bool(arguments.dry_run),
        ci_timeout=timeout,
    )


def main(argv: Sequence[str] | None = None) -> int:
    try:
        options = parse_arguments(argv)
        release(options)
    except ReleaseError as error:
        print(f"release: error: {error}", file=sys.stderr, flush=True)
        return 1
    except pipeline.PhaseFailure as error:
        print(f"release: error: {error}", file=sys.stderr, flush=True)
        return 1
    except subprocess.TimeoutExpired as error:
        print(f"release: error: {error.cmd[0]} timed out", file=sys.stderr, flush=True)
        return 1
    except KeyboardInterrupt:
        print("release: interrupted", file=sys.stderr, flush=True)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
