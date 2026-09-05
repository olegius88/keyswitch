#!/usr/bin/env python3
"""Unattended Linux release pipeline for KeySwitch.

The script runs the baseline KSLM/application contour described in
``docs/intent-model-runbook.md`` as one process: model provenance and
reproducibility evidence, strict typing, scoped 100%
coverage, detector gates, X11/tray end-to-end tests, the native Debian build,
its verifier and the packaged end-to-end test.
The workflows additionally run contextual-model training/engine replays,
native AT-SPI E2E and separate Windows jobs; these are not pipeline phases.
See ``docs/verification.md`` for the complete verification map.

Phases form a dependency graph and run concurrently.  A memory-aware scheduler
admits a phase only when the host has enough available RAM for its declared
peak on top of what already running phases may still claim. These estimates
reduce memory pressure but cannot guarantee against external load or an
underestimated peak. Every phase is recorded in a
machine-readable ``state.json`` while it runs, and the run ends with
``summary.json`` and ``SUMMARY.md`` so a reviewer (a person or an LLM) can check
the outcome after the fact without watching the terminal.

Typical use::

    python3 tools/release_pipeline.py start --profile release
    python3 tools/release_pipeline.py status
    python3 tools/release_pipeline.py wait

``start`` detaches the run from the calling terminal; the run survives the end
of the session that launched it.  ``run`` executes the same pipeline in the
foreground.  The script depends only on the Python standard library and the
tools already required by the documented release contour.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shlex
import signal
import struct
import subprocess
import sys
import threading
import time
import traceback
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from types import FrameType
from typing import IO, Final


PROJECT_ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DEFAULT_PIPELINE_ROOT: Final[Path] = PROJECT_ROOT / "dist" / "release-pipeline"
STATE_SCHEMA_VERSION: Final[int] = 1
STATE_FILE: Final[str] = "state.json"
SUMMARY_JSON: Final[str] = "summary.json"
SUMMARY_MARKDOWN: Final[str] = "SUMMARY.md"
PIPELINE_LOG: Final[str] = "pipeline.log"
PID_FILE: Final[str] = "pid"
LATEST_LINK: Final[str] = "latest"

XVFB_SCREEN: Final[str] = "-screen 0 1280x800x24"
XVFB_SCREEN_NORESET: Final[str] = "-screen 0 1280x800x24 -noreset"
MYPY_VERSION: Final[str] = "2.3.1"
NUITKA_VERSION: Final[str] = "4.2"
TYPING_ROOT: Final[Path] = PROJECT_ROOT / ".typing"
NUITKA_ROOT: Final[Path] = PROJECT_ROOT / ".nuitka"

MODEL_DIRECTORY: Final[Path] = PROJECT_ROOT / "model" / "intent_v1"
MODEL_CONFIG: Final[Path] = MODEL_DIRECTORY / "config.json"
MODEL_MANIFEST: Final[Path] = MODEL_DIRECTORY / "manifest.json"
MODEL_TEST_REPORT: Final[Path] = MODEL_DIRECTORY / "test-report.json"
MODEL_SOURCES: Final[Path] = MODEL_DIRECTORY / "sources"
MODEL_ENGLISH: Final[Path] = MODEL_SOURCES / "en_US.lm"
MODEL_RUSSIAN: Final[Path] = MODEL_SOURCES / "ru_RU.lm"
MODEL_ARTIFACT: Final[Path] = (
    PROJECT_ROOT / "src" / "keyswitch" / "resources" / "models" / "layout_intent_v1.ksm"
)
KSLM_HEADER: Final[struct.Struct] = struct.Struct("<4sHHIII32s")
KSLM_MAX_CONTAINER: Final[int] = 14 * 1024 * 1024
KSLM_MAX_MANIFEST: Final[int] = 1024 * 1024
KSLM_MAX_PAYLOAD: Final[int] = 12 * 1024 * 1024
KSLM_MAX_FINGERPRINTS: Final[int] = 1 << 20
JSON_READ_LIMIT: Final[int] = 64 * 1024 * 1024

# Same mapping that packaging/build-windows.ps1 enforces; the preseal receipt
# path is derived from the registry version at run time.
MODEL_TOOLCHAIN_PATHS: Final[Mapping[str, str]] = {
    "trainer_sha256": "tools/train_intent_model.py",
    "runtime_sha256": "src/keyswitch/intent_model.py",
    "detector_sha256": "src/keyswitch/detector.py",
    "protected_tokens_sha256": "src/keyswitch/resources/protected_tokens.txt",
    "layouts_sha256": "src/keyswitch/layouts.py",
    "language_model_sha256": "src/keyswitch/language_model.py",
    "evaluator_sha256": "tools/evaluate_intent_model.py",
    "preseal_generator_sha256": "tools/preseal_intent_holdout.py",
    "development_freezer_sha256": "tools/freeze_intent_development_corpus.py",
}

# Packages installed by the ``verify`` job of .github/workflows/tests.yml.
CI_APT_PACKAGES: Final[tuple[str, ...]] = (
    "at-spi2-core",
    "build-essential",
    "ccache",
    "dbus-x11",
    "desktop-file-utils",
    "file",
    "gir1.2-adw-1",
    "gir1.2-atspi-2.0",
    "gir1.2-gtk-4.0",
    "hunspell-en-us",
    "hunspell-ru",
    "libglib2.0-bin",
    "libhunspell-1.7-0",
    "libx11-6",
    "libxkbcommon0",
    "libxtst6",
    "lintian",
    "onboard-data",
    "patch",
    "patchelf",
    "python3-coverage",
    "python3-dbus",
    "python3-dev",
    "python3-gi",
    "python3-pip",
    "x11-xkb-utils",
    "xauth",
    "xvfb",
)
REQUIRED_COMMANDS: Final[tuple[str, ...]] = (
    "Xvfb",
    "cmp",
    "dbus-run-session",
    "desktop-file-validate",
    "dpkg",
    "dpkg-deb",
    "file",
    "gcc",
    "git",
    "ldd",
    "lintian",
    "patch",
    "patchelf",
    "python3",
    "realpath",
    "setxkbmap",
    "sha256sum",
    "stat",
    "timeout",
    "xvfb-run",
)

MODEL_DOCUMENTS_WITH_HASHES: Final[tuple[str, ...]] = (
    "model/intent_v1/MODEL_CARD.md",
    "model/intent_v1/MODEL_CARD.en.md",
    "CHANGELOG.md",
)
MODEL_DOCUMENTS_WITH_NAMESPACE: Final[tuple[str, ...]] = (
    "README.md",
    "README.en.md",
    "DESIGN.md",
    "docs/intent-model-runbook.md",
    "model/intent_v1/MODEL_CARD.md",
    "model/intent_v1/MODEL_CARD.en.md",
)

REPLAY_FILES: Final[tuple[str, ...]] = (
    "layout_intent_v1.ksm",
    "manifest.json",
    "test-report.json",
)
# One trainer replay holds ~8 GiB in the parent and forks one worker per CPU in
# its feature/scoring phases; two replays in parallel plus a desktop exhausted a
# 32 GiB host on 2026-09-02, so replays run one at a time under this budget.
REPLAY_MEMORY_MIB: Final[int] = 16000
REPLAY_POLL_SECONDS: Final[int] = 30
SCHEDULER_POLL_SECONDS: Final[float] = 5.0
TERMINATE_GRACE_SECONDS: Final[int] = 30
DEFAULT_MEMORY_RESERVE_MIB: Final[int] = 2048
FAILED_STATUSES: Final[frozenset[str]] = frozenset({"failed", "aborted"})


class PhaseFailure(Exception):
    """A phase failed with a reviewer-readable reason."""


class PipelineAborted(Exception):
    """The pipeline was interrupted by a termination signal."""


class UsageError(Exception):
    """Invalid command-line usage."""


# --------------------------------------------------------------------------
# Small typed helpers
# --------------------------------------------------------------------------


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def run_identifier(profile: str) -> str:
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return f"{stamp}-{profile}"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while True:
            chunk = stream.read(1 << 20)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def read_bounded(path: Path, limit: int, label: str) -> bytes:
    with path.open("rb") as stream:
        payload = stream.read(limit + 1)
    if len(payload) > limit:
        raise PhaseFailure(f"{label} exceeds {limit} bytes: {path}")
    return payload


def as_object(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise PhaseFailure(f"{label} must be a JSON object")
    result: dict[str, object] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            raise PhaseFailure(f"{label} has a non-string key")
        result[key] = item
    return result


def as_str(value: object, label: str) -> str:
    if not isinstance(value, str):
        raise PhaseFailure(f"{label} must be a string")
    return value


def as_bool(value: object, label: str) -> bool:
    if not isinstance(value, bool):
        raise PhaseFailure(f"{label} must be a boolean")
    return value


def as_int(value: object, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise PhaseFailure(f"{label} must be an integer")
    return value


def optional_str(value: object) -> str | None:
    return value if isinstance(value, str) else None


def optional_number(value: object) -> float | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)):
        return float(value)
    return None


def load_json_object(path: Path, label: str) -> dict[str, object]:
    if not path.is_file():
        raise PhaseFailure(f"{label} is missing: {path}")
    try:
        payload = json.loads(read_bounded(path, JSON_READ_LIMIT, label).decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PhaseFailure(f"{label} is not valid JSON: {error}") from error
    return as_object(payload, label)


def lookup(mapping: Mapping[str, object], *keys: str) -> object:
    current: object = dict(mapping)
    walked: list[str] = []
    for key in keys:
        walked.append(key)
        current = as_object(current, ".".join(walked[:-1]) or "document")
        if key not in current:
            raise PhaseFailure(f"missing JSON field: {'.'.join(walked)}")
        current = current[key]
    return current


def relative(path: Path) -> str:
    try:
        return str(path.relative_to(PROJECT_ROOT))
    except ValueError:
        return str(path)


def project_version() -> str:
    text = (PROJECT_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    match = re.search(r'^version = "([^"]+)"$', text, re.MULTILINE)
    if match is None:
        raise PhaseFailure("pyproject.toml does not declare a version")
    return match.group(1)


def debian_architecture() -> str:
    override = os.environ.get("DEB_HOST_ARCH", "")
    if override:
        return override
    return subprocess.run(
        ["dpkg", "--print-architecture"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def process_command_lines() -> list[str]:
    """Return the command lines of all visible processes (Linux /proc)."""

    lines: list[str] = []
    proc = Path("/proc")
    if not proc.is_dir():
        return lines
    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            raw = (entry / "cmdline").read_bytes()
        except OSError:
            continue
        if raw:
            lines.append(raw.replace(b"\0", b" ").decode("utf-8", "replace").strip())
    return lines


def session_rss_mib(session_ids: Sequence[int]) -> int:
    """Sum the memory of every process in the given sessions.

    Proportional set size (``smaps_rollup``) is used when the kernel exposes
    it, so pages shared by forked worker processes are counted once; plain RSS
    is the fallback.
    """

    wanted = set(session_ids)
    if not wanted:
        return 0
    sysconf = getattr(os, "sysconf", None)
    page_kib = (sysconf("SC_PAGE_SIZE") if sysconf is not None else 4096) // 1024
    total_kib = 0
    proc = Path("/proc")
    if not proc.is_dir():
        return 0
    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        # Fields after the parenthesised command name: state ppid pgrp session ...
        tail = stat.rpartition(")")[2].split()
        if len(tail) < 22:
            continue
        try:
            session = int(tail[3])
            rss_pages = int(tail[21])
        except ValueError:
            continue
        if session not in wanted:
            continue
        pss_kib = -1
        try:
            for line in (entry / "smaps_rollup").read_text(encoding="utf-8").splitlines():
                if line.startswith("Pss:"):
                    pss_kib = int(line.split()[1])
                    break
        except (OSError, ValueError, IndexError):
            pss_kib = -1
        total_kib += pss_kib if pss_kib >= 0 else rss_pages * page_kib
    return total_kib // 1024


def session_of(pid: int) -> int:
    """Return the session id of a process, or 0 when it cannot be read."""

    try:
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="replace")
    except OSError:
        return 0
    tail = stat.rpartition(")")[2].split()
    if len(tail) < 4:
        return 0
    try:
        return int(tail[3])
    except ValueError:
        return 0


def processes_with_argument(fragment: str) -> list[int]:
    """PIDs whose command line contains ``fragment`` (Linux /proc)."""

    pids: list[int] = []
    proc = Path("/proc")
    if not proc.is_dir():
        return pids
    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            raw = (entry / "cmdline").read_bytes()
        except OSError:
            continue
        if fragment.encode("utf-8") in raw:
            pids.append(int(entry.name))
    return pids


def kill_process_group(pid: int, signum: int) -> None:
    killpg = getattr(os, "killpg", None)
    getpgid = getattr(os, "getpgid", None)
    try:
        if killpg is None or getpgid is None:
            os.kill(pid, signum)
        else:
            killpg(getpgid(pid), signum)
    except ProcessLookupError:
        pass


def kill_signal() -> int:
    forced = getattr(signal, "SIGKILL", None)
    if isinstance(forced, signal.Signals):
        return int(forced)
    return int(signal.SIGTERM)


def available_cpus() -> int:
    affinity = getattr(os, "sched_getaffinity", None)
    if affinity is not None:
        return len(affinity(0))
    return os.cpu_count() or 1


def memory_info() -> dict[str, int]:
    result: dict[str, int] = {}
    meminfo = Path("/proc/meminfo")
    if not meminfo.is_file():
        return result
    for line in meminfo.read_text(encoding="utf-8").splitlines():
        key, _, rest = line.partition(":")
        if key in {"MemTotal", "MemAvailable", "SwapTotal", "SwapFree"}:
            amount = rest.strip().split()[0]
            result[f"{key.lower()}_mib"] = int(amount) // 1024
    return result


def git_output(*arguments: str) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        return ""
    return completed.stdout.strip()


def format_duration(seconds: float | None) -> str:
    if seconds is None:
        return "-"
    total = int(seconds)
    hours, remainder = divmod(total, 3600)
    minutes, secs = divmod(remainder, 60)
    if hours:
        return f"{hours}h{minutes:02d}m{secs:02d}s"
    if minutes:
        return f"{minutes}m{secs:02d}s"
    return f"{secs}s"


# --------------------------------------------------------------------------
# Options, state and logging
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Options:
    profile: str
    only: tuple[str, ...]
    skip: tuple[str, ...]
    start_from: str
    replays: int
    replay_dir: Path | None
    replay_strict: bool
    strict_report: Path | None
    workers: int
    jobs: int
    memory_reserve_mib: int
    timeout_scale: float
    fail_fast: bool
    pipeline_root: Path

    def to_argv(self) -> list[str]:
        argv = ["--profile", self.profile]
        if self.only:
            argv.extend(["--only", ",".join(self.only)])
        if self.skip:
            argv.extend(["--skip", ",".join(self.skip)])
        if self.start_from:
            argv.extend(["--from", self.start_from])
        argv.extend(["--replays", str(self.replays)])
        if self.replay_dir is not None:
            argv.extend(["--replay-dir", str(self.replay_dir)])
        if self.replay_strict:
            argv.append("--replay-strict")
        if self.strict_report is not None:
            argv.extend(["--strict-report", str(self.strict_report)])
        argv.extend(["--workers", str(self.workers)])
        argv.extend(["--jobs", str(self.jobs)])
        argv.extend(["--memory-reserve-mib", str(self.memory_reserve_mib)])
        argv.extend(["--timeout-scale", repr(self.timeout_scale)])
        if self.fail_fast:
            argv.append("--fail-fast")
        argv.extend(["--pipeline-root", str(self.pipeline_root)])
        return argv


@dataclass
class PhaseState:
    name: str
    title: str
    status: str = "pending"
    started_at: str | None = None
    finished_at: str | None = None
    duration_seconds: float | None = None
    log: str | None = None
    error: str | None = None
    waiting_reason: str | None = None
    observed_peak_rss_mib: int = 0
    facts: dict[str, object] = field(default_factory=dict)
    notes: list[str] = field(default_factory=list)
    log_tail: list[str] = field(default_factory=list)

    def to_json(self) -> dict[str, object]:
        return {
            "name": self.name,
            "title": self.title,
            "status": self.status,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "duration_seconds": self.duration_seconds,
            "log": self.log,
            "error": self.error,
            "waiting_reason": self.waiting_reason,
            "observed_peak_rss_mib": self.observed_peak_rss_mib,
            "facts": dict(self.facts),
            "notes": list(self.notes),
            "log_tail": list(self.log_tail),
        }


class PhaseLog:
    """Append-only log of one phase; also the stdout/stderr sink of commands."""

    def __init__(self, path: Path) -> None:
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)
        self._handle: IO[bytes] = path.open("ab")
        self._lock = threading.Lock()

    @property
    def handle(self) -> IO[bytes]:
        return self._handle

    def write(self, message: str) -> None:
        with self._lock:
            self._handle.write(f"[{utc_now()}] {message}\n".encode("utf-8"))
            self._handle.flush()

    def close(self) -> None:
        with self._lock:
            self._handle.close()

    def text(self) -> str:
        return self.path.read_bytes().decode("utf-8", "replace")

    def tail(self, lines: int = 40) -> list[str]:
        return [line.rstrip() for line in self.text().splitlines()[-lines:]]

    def contains(self, marker: str) -> bool:
        return marker in self.text()


class Context:
    """Run-wide services shared by every phase (thread-safe where needed)."""

    def __init__(self, run_dir: Path, options: Options, selected: Sequence[str]) -> None:
        self.run_dir = run_dir
        self.options = options
        self.selected = tuple(selected)
        self.stop_requested = False
        self.model_dir = run_dir / "model"
        self.artifacts_dir = run_dir / "artifacts"
        self.model_dir.mkdir(parents=True, exist_ok=True)
        self.artifacts_dir.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._processes: dict[str, subprocess.Popen[bytes]] = {}
        self._external_sessions: dict[str, list[int]] = {}

    def timeout(self, seconds: int) -> int:
        return max(60, int(seconds * self.options.timeout_scale))

    def replay_root(self) -> Path:
        if self.options.replay_dir is not None:
            return self.options.replay_dir
        return self.model_dir / "replays"

    def register(self, key: str, process: subprocess.Popen[bytes]) -> None:
        with self._lock:
            self._processes[key] = process

    def unregister(self, key: str) -> None:
        with self._lock:
            self._processes.pop(key, None)
            self._external_sessions.pop(key, None)

    def register_external(self, key: str, pids: Sequence[int]) -> None:
        """Track sessions of processes this run adopted but did not start."""

        sessions = [session for session in (session_of(pid) for pid in pids) if session > 0]
        with self._lock:
            self._external_sessions[key] = sessions

    def session_ids(self, phase: str) -> list[int]:
        with self._lock:
            own = [
                process.pid
                for key, process in self._processes.items()
                if key == phase or key.startswith(phase + "/")
            ]
            external = [
                session
                for key, sessions in self._external_sessions.items()
                if key == phase or key.startswith(phase + "/")
                for session in sessions
            ]
            return own + external

    def request_stop(self) -> None:
        self.stop_requested = True
        with self._lock:
            processes = list(self._processes.values())
        for process in processes:
            if process.poll() is None:
                kill_process_group(process.pid, int(signal.SIGTERM))

    def run_command(
        self,
        phase: str,
        log: PhaseLog,
        argv: Sequence[str],
        *,
        timeout: int,
        env: Mapping[str, str] | None = None,
        cwd: Path | None = None,
        stdout_path: Path | None = None,
        check: bool = True,
    ) -> int:
        if self.stop_requested:
            raise PipelineAborted()
        log.write("$ " + shlex.join(argv))
        merged = dict(os.environ)
        merged["PYTHONUNBUFFERED"] = "1"
        if env is not None:
            merged.update(env)
        stdout_handle: IO[bytes] | None = None
        if stdout_path is not None:
            stdout_handle = stdout_path.open("wb")
            log.write(f"stdout -> {stdout_path}")
        started = time.monotonic()
        try:
            process = subprocess.Popen(
                list(argv),
                cwd=cwd or PROJECT_ROOT,
                env=merged,
                stdin=subprocess.DEVNULL,
                stdout=stdout_handle if stdout_handle is not None else log.handle,
                stderr=log.handle,
                start_new_session=True,
            )
            self.register(phase, process)
            try:
                returncode = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                self._terminate(process)
                raise PhaseFailure(
                    f"{argv[0]} timed out after {timeout} seconds"
                ) from None
            finally:
                self.unregister(phase)
        finally:
            if stdout_handle is not None:
                stdout_handle.close()
        elapsed = time.monotonic() - started
        log.write(f"exit status {returncode} after {elapsed:.1f} s")
        if self.stop_requested:
            raise PipelineAborted()
        if check and returncode != 0:
            raise PhaseFailure(f"{shlex.join(argv[:2])} exited with status {returncode}")
        return returncode

    @staticmethod
    def _terminate(process: subprocess.Popen[bytes]) -> None:
        kill_process_group(process.pid, int(signal.SIGTERM))
        try:
            process.wait(timeout=TERMINATE_GRACE_SECONDS)
        except subprocess.TimeoutExpired:
            kill_process_group(process.pid, kill_signal())
            process.wait()

    @staticmethod
    def display_command(argv: Sequence[str], *, noreset: bool = False) -> list[str]:
        """Wrap a command exactly like CI: private D-Bus session plus Xvfb."""

        screen = XVFB_SCREEN_NORESET if noreset else XVFB_SCREEN
        return ["dbus-run-session", "--", "xvfb-run", "-a", "-s", screen, *argv]


PhaseRunner = Callable[[Context, PhaseLog, PhaseState], None]


@dataclass(frozen=True)
class PhaseSpec:
    name: str
    title: str
    runner: PhaseRunner
    depends: tuple[str, ...]
    timeout_seconds: int
    expected_seconds: int
    memory_mib: int
    lane: str = ""
    # Phases that measure wall-clock latency run alone: nothing else may be
    # running when they start, and nothing is admitted until they finish.
    exclusive: bool = False


# --------------------------------------------------------------------------
# Phase implementations
# --------------------------------------------------------------------------


def phase_environment(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    facts: dict[str, object] = {
        "python_version": platform.python_version(),
        "python_build": " ".join(platform.python_build()),
        "python_executable": sys.executable,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "available_cpus": available_cpus(),
        "memory": memory_info(),
        "display": os.environ.get("DISPLAY", ""),
        "session_type": os.environ.get("XDG_SESSION_TYPE", ""),
        "jobs": ctx.options.jobs,
        "memory_reserve_mib": ctx.options.memory_reserve_mib,
    }
    release = Path("/etc/os-release")
    if release.is_file():
        for line in release.read_text(encoding="utf-8").splitlines():
            if line.startswith("PRETTY_NAME="):
                facts["os"] = line.partition("=")[2].strip().strip('"')

    missing_commands = [
        name
        for name in REQUIRED_COMMANDS
        if subprocess.run(["which", name], capture_output=True, check=False).returncode != 0
    ]
    facts["missing_commands"] = missing_commands

    packages: dict[str, str] = {}
    query = subprocess.run(
        ["dpkg-query", "-W", "-f=${Package} ${Version}\\n", *CI_APT_PACKAGES],
        capture_output=True,
        text=True,
        check=False,
    )
    log.write(query.stdout.rstrip())
    if query.stderr:
        log.write(query.stderr.rstrip())
    for line in query.stdout.splitlines():
        name, _, version = line.partition(" ")
        if name:
            packages[name] = version
    missing_packages = [name for name in CI_APT_PACKAGES if name not in packages]
    facts["apt_packages"] = packages
    facts["missing_apt_packages"] = missing_packages

    problems: list[str] = []
    if missing_commands:
        problems.append("missing commands: " + ", ".join(missing_commands))
    if missing_packages:
        problems.append("missing apt packages: " + ", ".join(missing_packages))

    if "typecheck" in ctx.selected:
        facts["mypy"] = ensure_pip_tool(
            ctx,
            log,
            TYPING_ROOT,
            probe=["python3", "-m", "mypy", "--version"],
            expected=f"mypy {MYPY_VERSION}",
            installer=PROJECT_ROOT / "tools" / "install-typing-tools.sh",
        )
    if "build-deb" in ctx.selected:
        facts["nuitka"] = ensure_pip_tool(
            ctx,
            log,
            NUITKA_ROOT,
            probe=[
                "python3",
                "-c",
                "from nuitka.Version import getNuitkaVersion; "
                "print('nuitka', getNuitkaVersion())",
            ],
            expected=f"nuitka {NUITKA_VERSION}",
            installer=PROJECT_ROOT / "tools" / "install-build-tools.sh",
        )
    state.facts.update(facts)
    if problems:
        raise PhaseFailure("; ".join(problems))


def ensure_pip_tool(
    ctx: Context,
    log: PhaseLog,
    root: Path,
    *,
    probe: Sequence[str],
    expected: str,
    installer: Path,
) -> str:
    def probe_version() -> str:
        completed = subprocess.run(
            list(probe),
            cwd=PROJECT_ROOT,
            env={**os.environ, "PYTHONPATH": str(root)},
            capture_output=True,
            text=True,
            check=False,
        )
        output = completed.stdout.strip()
        return output.splitlines()[0] if output else ""

    found = probe_version()
    if found.startswith(expected):
        log.write(f"{root.name}: {found}")
        return found
    log.write(f"{root.name}: found {found or 'nothing'}, expected {expected}; installing")
    ctx.run_command(
        "environment", log, [str(installer), str(root)], timeout=ctx.timeout(1800)
    )
    found = probe_version()
    if not found.startswith(expected):
        raise PhaseFailure(f"{installer.name} did not provide {expected} in {root}")
    return found


def kslm_bounds(path: Path) -> dict[str, object]:
    data = read_bounded(path, KSLM_MAX_CONTAINER, "KSLM container")
    if len(data) < KSLM_HEADER.size:
        raise PhaseFailure("KSLM header is truncated")
    magic, schema, flags, manifest_length, payload_length, _crc, _digest = (
        KSLM_HEADER.unpack_from(data)
    )
    if magic != b"KSLM":
        raise PhaseFailure("KSLM magic is invalid")
    if schema != 4:
        raise PhaseFailure(f"KSLM schema {schema} is unsupported")
    if flags != 0:
        raise PhaseFailure("KSLM header flags are unsupported")
    if not 2 <= manifest_length <= KSLM_MAX_MANIFEST:
        raise PhaseFailure("KSLM embedded manifest exceeds the 1 MiB bound")
    if not 0 < payload_length <= KSLM_MAX_PAYLOAD:
        raise PhaseFailure("KSLM payload exceeds the 12 MiB bound")
    if len(data) != KSLM_HEADER.size + manifest_length + payload_length:
        raise PhaseFailure("KSLM header lengths do not match the container")
    embedded = as_object(
        json.loads(
            data[KSLM_HEADER.size : KSLM_HEADER.size + manifest_length].decode("utf-8")
        ),
        "KSLM embedded manifest",
    )
    fingerprints = as_int(embedded.get("supported_fingerprint_count"), "fingerprint count")
    dimension = as_int(embedded.get("dimension"), "dimension")
    if not 0 <= fingerprints <= KSLM_MAX_FINGERPRINTS:
        raise PhaseFailure("KSLM fingerprint count exceeds the 2^20 bound")
    if dimension <= 0 or payload_length != dimension * 2 + fingerprints * 8:
        raise PhaseFailure("KSLM payload shape does not match its embedded manifest")
    return {
        "bytes": len(data),
        "schema": schema,
        "dimension": dimension,
        "fingerprints": fingerprints,
        "embedded_model_version": embedded.get("model_version"),
    }


@dataclass(frozen=True)
class ModelIdentity:
    config: dict[str, object]
    manifest: dict[str, object]
    registry_path: Path
    receipt_path: Path
    hard_negative_path: Path
    split_namespace: str
    artifact_version: str
    artifact_sha256: str


def model_identity() -> ModelIdentity:
    config = load_json_object(MODEL_CONFIG, "model config")
    manifest = load_json_object(MODEL_MANIFEST, "model manifest")
    registry_relative = as_str(
        lookup(config, "sealed_evaluation", "registry_path"), "registry path"
    )
    registry_path = PROJECT_ROOT / registry_relative
    match = re.search(r"seal-registry-v(\d+)\.json$", registry_relative)
    if match is None:
        raise PhaseFailure(f"unexpected registry path: {registry_relative}")
    receipt_path = MODEL_DIRECTORY / f"holdout-v{match.group(1)}-preseal.json"
    hard_negative_path = PROJECT_ROOT / as_str(
        lookup(config, "hard_negative_development", "source", "path"),
        "hard-negative path",
    )
    return ModelIdentity(
        config=config,
        manifest=manifest,
        registry_path=registry_path,
        receipt_path=receipt_path,
        hard_negative_path=hard_negative_path,
        split_namespace=as_str(
            lookup(config, "sealed_evaluation", "split_namespace"), "split namespace"
        ),
        artifact_version=as_str(
            manifest.get("artifact_model_version"), "artifact version"
        ),
        artifact_sha256=as_str(manifest.get("artifact_sha256"), "artifact sha256"),
    )


def phase_model_inputs(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    problems: list[str] = []
    ctx.run_command(
        state.name,
        log,
        ["sha256sum", "--check", "SHA256SUMS"],
        cwd=MODEL_SOURCES,
        timeout=ctx.timeout(600),
    )
    identity = model_identity()
    config, manifest = identity.config, identity.manifest
    facts: dict[str, object] = {
        "artifact_model_version": identity.artifact_version,
        "artifact_sha256": identity.artifact_sha256,
        "split_namespace": identity.split_namespace,
        "registry": relative(identity.registry_path),
        "preseal_receipt": relative(identity.receipt_path),
        "hard_negative_source": relative(identity.hard_negative_path),
        "config_sha256": sha256_file(MODEL_CONFIG),
        "manifest_sha256": sha256_file(MODEL_MANIFEST),
        "test_report_sha256": sha256_file(MODEL_TEST_REPORT),
    }

    # Artifact identity.
    if not MODEL_ARTIFACT.is_file():
        raise PhaseFailure(f"bundled artifact is missing: {MODEL_ARTIFACT}")
    if sha256_file(MODEL_ARTIFACT) != identity.artifact_sha256:
        problems.append("manifest.artifact_sha256 does not match the bundled KSLM file")
    provenance = as_str(manifest.get("build_provenance_sha256"), "build provenance")
    if identity.artifact_version != f"intent-v1-{provenance[:12]}":
        problems.append(
            "artifact_model_version is not derived from build_provenance_sha256"
        )
    try:
        bounds = kslm_bounds(MODEL_ARTIFACT)
        facts["kslm"] = bounds
        if bounds["embedded_model_version"] != identity.artifact_version:
            problems.append("KSLM embedded model_version differs from the manifest")
    except PhaseFailure as error:
        problems.append(str(error))

    # Internal gates.
    if not as_bool(manifest.get("quality_gates_passed"), "manifest gates"):
        problems.append("manifest.quality_gates_passed is not true")
    report = load_json_object(MODEL_TEST_REPORT, "test report")
    if not as_bool(report.get("quality_gates_passed"), "test-report gates"):
        problems.append("test-report.quality_gates_passed is not true")
    training = as_object(manifest.get("training"), "manifest.training")
    history = training.get("history")
    facts["training"] = {
        "best_epoch": training.get("best_epoch"),
        "epochs_executed": len(history) if isinstance(history, list) else None,
        "nonzero_weights": training.get("nonzero_weights"),
    }

    # Registry and sealed evaluation.
    if not identity.registry_path.is_file():
        problems.append(f"seal registry is missing: {relative(identity.registry_path)}")
    else:
        registry = load_json_object(identity.registry_path, "seal registry")
        sealed = as_object(manifest.get("sealed_evaluation"), "manifest.sealed_evaluation")
        for key in (
            "schema_version",
            "split_namespace",
            "candidate_sha256",
            "config_sha256",
            "candidate_dataset_sha256",
        ):
            if registry.get(key) != sealed.get(key):
                problems.append(f"seal registry field differs from manifest: {key}")
        if sealed.get("registry_path") != relative(identity.registry_path):
            problems.append("manifest.sealed_evaluation.registry_path differs from config")
        if sealed.get("registry_sha256") != sha256_file(identity.registry_path):
            problems.append(
                "manifest.sealed_evaluation.registry_sha256 differs from the file"
            )
        if registry.get("split_namespace") != identity.split_namespace:
            problems.append("seal registry split_namespace differs from config")
        facts["candidate_sha256"] = registry.get("candidate_sha256")
    if manifest.get("split_namespace") != identity.split_namespace:
        problems.append("manifest.split_namespace differs from config")
    if manifest.get("config_sha256") != facts["config_sha256"]:
        problems.append("manifest.config_sha256 differs from config.json")

    # Toolchain provenance: the working tree must still be the trained one.
    toolchain = as_object(manifest.get("toolchain"), "manifest.toolchain")
    toolchain_paths = dict(MODEL_TOOLCHAIN_PATHS)
    toolchain_paths["preseal_receipt_sha256"] = relative(identity.receipt_path)
    drifted: list[str] = []
    for field_name, relative_path in toolchain_paths.items():
        path = PROJECT_ROOT / relative_path
        if not path.is_file():
            drifted.append(f"{relative_path} (missing)")
            continue
        if toolchain.get(field_name) != sha256_file(path):
            drifted.append(relative_path)
    facts["toolchain_drift"] = drifted
    if drifted:
        problems.append("toolchain files differ from manifest: " + ", ".join(drifted))
    facts["toolchain_python"] = {
        "manifest": toolchain.get("python_version"),
        "host": platform.python_version(),
    }
    if toolchain.get("python_version") != platform.python_version():
        state.notes.append(
            "host Python differs from the training Python; byte-identical replay "
            "is only promised in the reference environment"
        )

    # Frozen hard-negative development source.
    source = as_object(
        lookup(config, "hard_negative_development", "source"), "hard-negative source"
    )
    if not identity.hard_negative_path.is_file():
        problems.append("frozen hard-negative development source is missing")
    else:
        if source.get("sha256") != sha256_file(identity.hard_negative_path):
            problems.append("hard-negative development source sha256 differs from config")
        if source.get("bytes") != identity.hard_negative_path.stat().st_size:
            problems.append("hard-negative development source size differs from config")

    # Model-blind preseal receipt.
    if not identity.receipt_path.is_file():
        problems.append(f"preseal receipt is missing: {relative(identity.receipt_path)}")
    else:
        receipt = load_json_object(identity.receipt_path, "preseal receipt")
        checks: list[tuple[str, bool]] = [
            ("model_loaded == false", receipt.get("model_loaded") is False),
            ("metrics_evaluated == false", receipt.get("metrics_evaluated") is False),
            (
                "overlap development/holdout == 0",
                lookup(receipt, "overlap_counts", "development_holdout") == 0,
            ),
            (
                "overlap sealed/holdout == 0",
                lookup(receipt, "overlap_counts", "sealed_holdout") == 0,
            ),
            (
                "development signatures > 0",
                as_int(lookup(receipt, "development", "signature_count"), "count") > 0,
            ),
            (
                "holdout signatures > 0",
                as_int(lookup(receipt, "holdout", "signature_count"), "count") > 0,
            ),
            (
                "receipt frozen source == config source",
                lookup(receipt, "development", "frozen_source", "sha256")
                == source.get("sha256"),
            ),
            (
                "receipt holdout corpus == config external holdout",
                lookup(receipt, "holdout", "corpus_sha256")
                == lookup(
                    config, "external_evaluation", "unknown_typo_holdout_corpus_sha256"
                ),
            ),
            (
                "receipt development corpus == config development corpus",
                lookup(receipt, "development", "corpus_sha256")
                == lookup(
                    config,
                    "external_evaluation",
                    "unknown_typo_development_corpus_sha256",
                ),
            ),
        ]
        facts["preseal_receipt_checks"] = {name: passed for name, passed in checks}
        for name, passed in checks:
            if not passed:
                problems.append(f"preseal receipt check failed: {name}")

    state.facts.update(facts)
    for problem in problems:
        log.write("PROBLEM: " + problem)
    if problems:
        raise PhaseFailure("; ".join(problems))


def model_tool_arguments() -> list[str]:
    return [
        "--config",
        str(MODEL_CONFIG),
        "--en-model",
        str(MODEL_ENGLISH),
        "--ru-model",
        str(MODEL_RUSSIAN),
    ]


def python_env() -> dict[str, str]:
    return {"PYTHONPATH": str(PROJECT_ROOT / "src")}


def phase_model_development_replay(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    identity = model_identity()
    output = ctx.model_dir / "unknown-typo-development-replay.json"
    ctx.run_command(
        state.name,
        log,
        [
            "python3",
            str(PROJECT_ROOT / "tools" / "freeze_intent_development_corpus.py"),
            *model_tool_arguments(),
            "--output",
            str(output),
        ],
        env=python_env(),
        timeout=ctx.timeout(3 * 3600),
    )
    identical = output.read_bytes() == identity.hard_negative_path.read_bytes()
    state.facts.update(
        {
            "frozen_source": relative(identity.hard_negative_path),
            "frozen_sha256": sha256_file(identity.hard_negative_path),
            "replay_sha256": sha256_file(output),
            "byte_identical": identical,
        }
    )
    if not identical:
        raise PhaseFailure(
            "development corpus replay is not byte-identical to the frozen source"
        )


def phase_model_preseal_replay(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    identity = model_identity()
    output = ctx.model_dir / "holdout-preseal-replay.json"
    ctx.run_command(
        state.name,
        log,
        [
            "python3",
            str(PROJECT_ROOT / "tools" / "preseal_intent_holdout.py"),
            *model_tool_arguments(),
        ],
        env=python_env(),
        stdout_path=output,
        timeout=ctx.timeout(3 * 3600),
    )
    identical = output.read_bytes() == identity.receipt_path.read_bytes()
    state.facts.update(
        {
            "receipt": relative(identity.receipt_path),
            "receipt_sha256": sha256_file(identity.receipt_path),
            "replay_sha256": sha256_file(output),
            "byte_identical": identical,
        }
    )
    if not identical:
        raise PhaseFailure(
            "preseal receipt replay is not byte-identical to the stored receipt"
        )


def strict_report_facts(
    report_path: Path, expected_sha256: str, expected_version: str
) -> tuple[dict[str, object], list[str]]:
    report = load_json_object(report_path, "strict report")
    gates = as_object(report.get("strict_gates"), "strict_gates")
    failed = sorted(name for name, value in gates.items() if value is not True)
    passed = as_bool(report.get("strict_passed"), "strict_passed")
    model = as_object(report.get("model"), "strict report model")
    facts: dict[str, object] = {
        "report": str(report_path),
        "report_sha256": sha256_file(report_path),
        "strict_passed": passed,
        "gate_count": len(gates),
        "failed_gates": failed,
        "model_version": model.get("version"),
        "model_checksum": model.get("checksum"),
        "performance": report.get("performance"),
    }
    problems: list[str] = []
    if not passed:
        problems.append("strict_passed is not true")
    if failed:
        problems.append("failed strict gates: " + ", ".join(failed))
    if model.get("checksum") != expected_sha256:
        problems.append("strict report checksum differs from the evaluated artifact")
    if model.get("version") != expected_version:
        problems.append("strict report model version differs from the manifest")
    facts["problems"] = list(problems)
    return facts, problems


def run_strict_evaluator(
    ctx: Context,
    phase: str,
    log: PhaseLog,
    *,
    artifact: Path,
    manifest: Path,
    output: Path,
) -> None:
    ctx.run_command(
        phase,
        log,
        [
            "python3",
            str(PROJECT_ROOT / "tools" / "evaluate_intent_model.py"),
            *model_tool_arguments(),
            "--artifact",
            str(artifact),
            "--manifest",
            str(manifest),
            "--strict",
        ],
        env=python_env(),
        stdout_path=output,
        timeout=ctx.timeout(4 * 3600),
        check=False,
    )


def phase_model_strict(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    identity = model_identity()
    output = ctx.model_dir / "strict.json"
    provided = ctx.options.strict_report
    if provided is not None and provided.is_file():
        facts, problems = strict_report_facts(
            provided, identity.artifact_sha256, identity.artifact_version
        )
        verified = (
            ctx.run_command(
                state.name,
                log,
                [
                    "python3",
                    str(PROJECT_ROOT / "tools" / "verify_intent_strict_report.py"),
                    "--report",
                    str(provided),
                ],
                timeout=ctx.timeout(600),
                check=False,
            )
            == 0
        )
        if not problems and verified:
            output.write_bytes(provided.read_bytes())
            facts["reused_from"] = str(provided)
            state.notes.append(f"reused existing strict report {provided}")
            state.facts.update(facts)
            return
        log.write(
            "provided strict report is not bound to the current tree, re-running: "
            f"{problems or 'verifier rejected it'}"
        )
    run_strict_evaluator(
        ctx,
        state.name,
        log,
        artifact=MODEL_ARTIFACT,
        manifest=MODEL_MANIFEST,
        output=output,
    )
    facts, problems = strict_report_facts(
        output, identity.artifact_sha256, identity.artifact_version
    )
    state.facts.update(facts)
    if problems:
        raise PhaseFailure("; ".join(problems))


def replay_outputs_present(directory: Path) -> bool:
    return all((directory / name).is_file() for name in REPLAY_FILES)


def replay_process_ids(directory: Path) -> list[int]:
    return processes_with_argument(str(directory / "layout_intent_v1.ksm"))


def replay_in_progress(directory: Path) -> bool:
    return bool(replay_process_ids(directory))


def replay_work_remaining(ctx: Context) -> int:
    """Replays that still have to run or finish (adopted running ones included)."""

    root = ctx.replay_root()
    return sum(
        1
        for label in ("a", "b")[: ctx.options.replays]
        if not replay_outputs_present(root / label)
    )


def replay_launch_count(ctx: Context) -> int:
    """How many replays the phase would have to start itself (not adopt)."""

    root = ctx.replay_root()
    return sum(
        1
        for label in ("a", "b")[: ctx.options.replays]
        if not replay_outputs_present(root / label) and not replay_in_progress(root / label)
    )


def run_one_replay(ctx: Context, log: PhaseLog, phase: str, directory: Path) -> str:
    """Adopt or start one replay and block until its outputs exist.

    Replays run strictly one at a time: a single trainer already needs most of
    the RAM of the reference host once it forks its worker pool.
    """

    label = directory.name
    key = f"{phase}/{label}"
    directory.mkdir(parents=True, exist_ok=True)
    if replay_outputs_present(directory):
        log.write(f"replay {label}: adopting finished outputs in {directory}")
        return "finished"
    process: subprocess.Popen[bytes] | None = None
    handles: list[IO[bytes]] = []
    mode = "adopted"
    if replay_in_progress(directory):
        ctx.register_external(key, replay_process_ids(directory))
        log.write(f"replay {label}: adopting the running trainer for {directory}")
    else:
        for name in REPLAY_FILES:
            if (directory / name).exists():
                raise PhaseFailure(f"replay {label} has a partial output set in {directory}")
        mode = "launched"
        stdout_handle = (directory / "train-stdout.json").open("wb")
        stderr_handle = (directory / "train.log").open("ab")
        handles.extend([stdout_handle, stderr_handle])
        argv = [
            "python3",
            str(PROJECT_ROOT / "tools" / "train_intent_model_release.py"),
            "--workers",
            str(ctx.options.workers),
            "--artifact",
            str(directory / "layout_intent_v1.ksm"),
            "--manifest",
            str(directory / "manifest.json"),
            "--test-report",
            str(directory / "test-report.json"),
        ]
        log.write(f"replay {label}: $ " + shlex.join(argv))
        process = subprocess.Popen(
            argv,
            cwd=PROJECT_ROOT,
            env={**os.environ, **python_env(), "PYTHONUNBUFFERED": "1"},
            stdin=subprocess.DEVNULL,
            stdout=stdout_handle,
            stderr=stderr_handle,
            start_new_session=True,
        )
        ctx.register(key, process)
    deadline = time.monotonic() + ctx.timeout(8 * 3600)
    last_progress = ""
    try:
        while True:
            if ctx.stop_requested:
                raise PipelineAborted()
            if process is not None:
                status = process.poll()
                if status is not None:
                    if status != 0:
                        raise PhaseFailure(f"replay {label} exited with status {status}")
                    if not replay_outputs_present(directory):
                        raise PhaseFailure(f"replay {label} finished without complete outputs")
                    break
            elif replay_outputs_present(directory):
                break
            elif not replay_in_progress(directory):
                raise PhaseFailure(
                    f"replay {label} is neither finished nor running in {directory}; "
                    "check the kernel log for an OOM kill of the trainer or its session"
                )
            if time.monotonic() > deadline:
                raise PhaseFailure(f"replay {label} did not finish before the timeout")
            train_log = directory / "train.log"
            if train_log.is_file():
                lines = train_log.read_bytes().decode("utf-8", "replace").splitlines()
                if lines and lines[-1].strip() != last_progress:
                    last_progress = lines[-1].strip()
                    log.write(f"replay {label}: {last_progress}")
            time.sleep(REPLAY_POLL_SECONDS)
    except BaseException:
        if process is not None and process.poll() is None:
            kill_process_group(process.pid, int(signal.SIGTERM))
        raise
    finally:
        ctx.unregister(key)
        for handle in handles:
            handle.close()
    return mode


def phase_model_replays(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    count = ctx.options.replays
    if count <= 0:
        state.notes.append("replays disabled with --replays 0")
        state.facts["replays"] = 0
        return
    root = ctx.replay_root()
    root.mkdir(parents=True, exist_ok=True)
    labels = ["a", "b"][:count]
    modes: dict[str, str] = {}
    for label in labels:
        modes[label] = run_one_replay(ctx, log, state.name, root / label)
        state.facts["modes"] = dict(modes)

    official = {
        "layout_intent_v1.ksm": MODEL_ARTIFACT,
        "manifest.json": MODEL_MANIFEST,
        "test-report.json": MODEL_TEST_REPORT,
    }
    hashes: dict[str, dict[str, str]] = {"official": {}}
    for name, path in official.items():
        hashes["official"][name] = sha256_file(path)
    for label in labels:
        hashes[label] = {name: sha256_file(root / label / name) for name in REPLAY_FILES}
    state.facts["sha256"] = hashes
    mismatches = [
        f"{label}/{name}"
        for label in labels
        for name in REPLAY_FILES
        if hashes[label][name] != hashes["official"][name]
    ]
    state.facts["byte_identical"] = not mismatches
    state.facts["replays"] = count
    state.facts["replay_root"] = str(root)
    if mismatches:
        raise PhaseFailure(
            "replay outputs differ from the official files: " + ", ".join(mismatches)
        )


def phase_model_replay_strict(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    if ctx.options.replays <= 0:
        state.status = "skipped"
        state.notes.append("replays disabled with --replays 0")
        return
    if not ctx.options.replay_strict:
        state.status = "skipped"
        state.notes.append("replay strict evaluation not requested (--replay-strict)")
        return
    identity = model_identity()
    directory = ctx.replay_root() / "a"
    if not replay_outputs_present(directory):
        raise PhaseFailure(f"replay a outputs are missing in {directory}")
    output = ctx.model_dir / "strict-replay-a.json"
    run_strict_evaluator(
        ctx,
        state.name,
        log,
        artifact=directory / "layout_intent_v1.ksm",
        manifest=directory / "manifest.json",
        output=output,
    )
    facts, problems = strict_report_facts(
        output, identity.artifact_sha256, identity.artifact_version
    )
    state.facts.update(facts)
    if problems:
        raise PhaseFailure("; ".join(problems))


def phase_typecheck(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    ctx.run_command(
        state.name,
        log,
        [str(PROJECT_ROOT / "tools" / "typecheck.sh")],
        env={"KEYSWITCH_TYPING_ROOT": str(TYPING_ROOT)},
        timeout=ctx.timeout(3600),
    )
    match = re.search(r"Success: no issues found in (\d+) source files", log.text())
    if match is None:
        raise PhaseFailure("mypy did not report a clean success line")
    state.facts["source_files"] = int(match.group(1))


def phase_coverage(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    ctx.run_command(
        state.name,
        log,
        ctx.display_command([str(PROJECT_ROOT / "tests" / "run_coverage.sh")]),
        timeout=ctx.timeout(2 * 3600),
    )
    text = log.text()
    ran = re.search(r"^Ran (\d+) tests? in ([\d.]+)s", text, re.MULTILINE)
    total = re.search(r"^TOTAL\s+.*?\s(\d+)%\s*$", text, re.MULTILINE)
    if ran is None or total is None:
        raise PhaseFailure("coverage output lacks the unittest or TOTAL summary line")
    state.facts.update(
        {
            "tests": int(ran.group(1)),
            "test_seconds": float(ran.group(2)),
            "coverage_percent": int(total.group(1)),
        }
    )
    if int(total.group(1)) != 100:
        raise PhaseFailure(f"coverage is {total.group(1)}%, 100% is required")


def phase_detector_gates(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    output = ctx.model_dir / "detector.json"
    ctx.run_command(
        state.name,
        log,
        [
            "python3",
            str(PROJECT_ROOT / "tools" / "evaluate_detector.py"),
            "--sample",
            "10000",
            "--dictionary-sample",
            "10000",
            "--strict",
        ],
        env=python_env(),
        stdout_path=output,
        timeout=ctx.timeout(2 * 3600),
    )
    payload = load_json_object(output, "detector report")
    failures = payload.get("curated_failures")
    samples = payload.get("sample_failures")
    state.facts.update(
        {
            "report": str(output),
            "curated_samples": payload.get("curated_samples"),
            "curated_failures": len(failures) if isinstance(failures, list) else failures,
            "sample_failures": len(samples) if isinstance(samples, list) else samples,
        }
    )


def require_markers(log: PhaseLog, state: PhaseState, markers: Sequence[str]) -> None:
    state.facts["markers"] = list(markers)
    for marker in markers:
        if not log.contains(marker):
            raise PhaseFailure(f"expected success marker {marker} was not printed")


def phase_e2e_x11(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    script = (
        "setxkbmap -layout us,ru && "
        f"GTK_A11Y=none PYTHONPATH={shlex.quote(str(PROJECT_ROOT / 'src'))} "
        f"python3 {shlex.quote(str(PROJECT_ROOT / 'tests' / 'e2e_x11.py'))}"
    )
    ctx.run_command(
        state.name,
        log,
        ctx.display_command(["bash", "-c", script], noreset=True),
        timeout=ctx.timeout(1800),
    )
    require_markers(log, state, ("E2E_OK", "MENU_LAYOUT_SELECTION_E2E_OK"))


def phase_e2e_tray(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    ctx.run_command(
        state.name,
        log,
        [
            "dbus-run-session",
            "--",
            "env",
            f"PYTHONPATH={PROJECT_ROOT / 'src'}",
            "python3",
            str(PROJECT_ROOT / "tests" / "e2e_tray_menu.py"),
        ],
        timeout=ctx.timeout(1800),
    )
    require_markers(log, state, ("TRAY_MENU_E2E_OK",))


def package_path(ctx: Context) -> Path:
    return ctx.artifacts_dir / f"keyswitch_{project_version()}_{debian_architecture()}.deb"


def phase_build_deb(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    package = package_path(ctx)
    env = {"KEYSWITCH_NUITKA_ROOT": str(NUITKA_ROOT)}
    strict_report = ctx.model_dir / "strict.json"
    if strict_report.is_file():
        # build-deb.sh re-verifies the report against the current tree with
        # tools/verify_intent_strict_report.py before skipping its own run.
        env["KEYSWITCH_INTENT_STRICT_REPORT"] = str(strict_report)
        state.notes.append("build-deb reuses the verified strict report of this run")
    ctx.run_command(
        state.name,
        log,
        [str(PROJECT_ROOT / "packaging" / "build-deb.sh"), str(ctx.artifacts_dir)],
        env=env,
        timeout=ctx.timeout(6 * 3600),
    )
    if not package.is_file():
        raise PhaseFailure(f"build-deb.sh did not produce {package}")
    identity = model_identity()
    strict = ctx.artifacts_dir / "keyswitch-intent-evaluation.json"
    facts: dict[str, object] = {
        "package": str(package),
        "package_sha256": sha256_file(package),
        "package_bytes": package.stat().st_size,
    }
    if strict.is_file():
        strict_facts, _problems = strict_report_facts(
            strict, identity.artifact_sha256, identity.artifact_version
        )
        facts["strict_report"] = strict_facts
    state.facts.update(facts)


def phase_verify_deb(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    package = package_path(ctx)
    if not package.is_file():
        raise PhaseFailure(f"package to verify is missing: {package}")
    ctx.run_command(
        state.name,
        log,
        [str(PROJECT_ROOT / "tools" / "verify-native-deb.sh"), str(package)],
        timeout=ctx.timeout(1800),
    )
    require_markers(log, state, ("NATIVE_DEB_OK",))
    ctx.run_command(
        state.name,
        log,
        [
            "desktop-file-validate",
            str(PROJECT_ROOT / "packaging" / "io.github.olegius88.KeySwitch.desktop"),
        ],
        timeout=ctx.timeout(300),
    )
    ctx.run_command(
        state.name,
        log,
        ["lintian", "--fail-on", "error", str(package)],
        timeout=ctx.timeout(1800),
    )
    state.facts.update({"package": str(package), "package_sha256": sha256_file(package)})


def phase_e2e_native(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    package = package_path(ctx)
    if not package.is_file():
        raise PhaseFailure(f"package for the native E2E is missing: {package}")
    script = (
        "setxkbmap -layout us,ru && GTK_USE_PORTAL=0 "
        f"{shlex.quote(str(PROJECT_ROOT / 'tools' / 'run-native-e2e.sh'))} \"$1\""
    )
    ctx.run_command(
        state.name,
        log,
        ctx.display_command(["bash", "-c", script, "_", str(package)], noreset=True),
        timeout=ctx.timeout(1800),
    )
    require_markers(log, state, ("NATIVE_E2E_OK", "NATIVE_LEARNING_PROMPT_E2E_OK"))


def changelog_sections(text: str) -> dict[str, list[str]]:
    sections: dict[str, list[str]] = {}
    current = ""
    for line in text.splitlines():
        if line.startswith("## "):
            current = line[3:].strip()
            sections.setdefault(current, [])
        elif current and line.startswith("- "):
            sections[current].append(line)
    return sections


def phase_release_metadata(ctx: Context, log: PhaseLog, state: PhaseState) -> None:
    problems: list[str] = []
    release_problems: list[str] = []
    version = project_version()
    facts: dict[str, object] = {"version": version}

    module_text = (PROJECT_ROOT / "src" / "keyswitch" / "__init__.py").read_text("utf-8")
    module_match = re.search(r'^__version__ = "([^"]+)"$', module_text, re.MULTILINE)
    module_version = module_match.group(1) if module_match is not None else ""
    man_text = (PROJECT_ROOT / "packaging" / "keyswitch.1").read_text("utf-8")
    man_match = re.search(r'"KeySwitch ([^"]+)"', man_text)
    man_version = man_match.group(1) if man_match is not None else ""
    notes_text = (PROJECT_ROOT / "RELEASE_NOTES.md").read_text("utf-8")
    notes_match = re.match(r"# KeySwitch (\S+)", notes_text)
    notes_version = notes_match.group(1) if notes_match is not None else ""
    changelog = changelog_sections((PROJECT_ROOT / "CHANGELOG.md").read_text("utf-8"))
    changelog_has_version = any(
        heading.split(" ")[0] == version for heading in changelog if heading != "Unreleased"
    )
    unreleased = changelog.get("Unreleased", [])
    facts["versions"] = {
        "pyproject": version,
        "module": module_version,
        "man_page": man_version,
        "release_notes": notes_version,
        "changelog_has_section": changelog_has_version,
        "changelog_unreleased_entries": len(unreleased),
    }
    if module_version != version:
        problems.append(f"src/keyswitch/__init__.py version {module_version!r} != {version!r}")
    if man_version != version:
        problems.append(f"packaging/keyswitch.1 version {man_version!r} != {version!r}")
    if notes_version != version:
        problems.append(f"RELEASE_NOTES.md version {notes_version!r} != {version!r}")
    if not changelog_has_version:
        problems.append(f"CHANGELOG.md has no section for {version}")
    if unreleased:
        release_problems.append(
            f"CHANGELOG.md keeps {len(unreleased)} entries under Unreleased; "
            "move them under the release version before tagging"
        )

    ctx.run_command(state.name, log, ["git", "diff", "--check"], timeout=ctx.timeout(300))

    head = git_output("rev-parse", "HEAD")
    dirty = [line for line in git_output("status", "--porcelain").splitlines() if line.strip()]
    tag_commit = git_output("rev-parse", "--verify", "--quiet", f"v{version}^{{commit}}")
    facts["git"] = {
        "head": head,
        "dirty_files": len(dirty),
        "tag": f"v{version}",
        "tag_commit": tag_commit or None,
    }
    if tag_commit and (tag_commit != head or dirty):
        release_problems.append(
            f"v{version} is already tagged at {tag_commit[:12]} while the tree differs "
            f"(HEAD {head[:12]}, {len(dirty)} dirty paths); bump the version for a new release"
        )

    identity = model_identity()
    stale_docs: list[str] = []
    for document in MODEL_DOCUMENTS_WITH_HASHES:
        text = (PROJECT_ROOT / document).read_text("utf-8")
        if identity.artifact_version not in text or identity.artifact_sha256 not in text:
            stale_docs.append(f"{document} (artifact version/sha256)")
    for document in MODEL_DOCUMENTS_WITH_NAMESPACE:
        text = (PROJECT_ROOT / document).read_text("utf-8")
        if identity.split_namespace not in text:
            stale_docs.append(f"{document} (split namespace)")
    facts["stale_model_documents"] = stale_docs
    if stale_docs:
        release_problems.append(
            "documents do not mention the current model identity: " + ", ".join(stale_docs)
        )

    attributes = (PROJECT_ROOT / ".gitattributes").read_text("utf-8").splitlines()
    missing_attributes = [
        relative(path)
        for path in (identity.registry_path, identity.receipt_path, identity.hard_negative_path)
        if f"{relative(path)} text eol=lf" not in attributes
    ]
    facts["missing_gitattributes"] = missing_attributes
    if missing_attributes:
        release_problems.append(
            ".gitattributes lacks eol=lf pins for: " + ", ".join(missing_attributes)
        )

    model_status = [
        line
        for line in git_output(
            "status", "--porcelain", "--", "model/intent_v1", "src/keyswitch/resources/models"
        ).splitlines()
        if line.strip()
    ]
    facts["uncommitted_model_paths"] = model_status
    if model_status:
        state.notes.append(
            f"{len(model_status)} model paths are modified or untracked; they must be "
            "committed together as one release change"
        )

    state.facts.update(facts)
    if ctx.options.profile == "release":
        problems.extend(release_problems)
    else:
        state.notes.extend("release: " + item for item in release_problems)
    for problem in problems:
        log.write("PROBLEM: " + problem)
    if problems:
        raise PhaseFailure("; ".join(problems))


# Memory estimates are conservative peaks (MiB): observed PSS on the reference
# host on 2026-09-02 plus headroom. Every run records ``observed_peak_rss_mib``
# per phase so the table can be recalibrated.
PHASES: Final[tuple[PhaseSpec, ...]] = (
    PhaseSpec(
        "environment",
        "Host, packages and pinned tools",
        phase_environment,
        (),
        1800,
        60,
        300,
    ),
    PhaseSpec(
        "model-inputs",
        "Model provenance and internal gates",
        phase_model_inputs,
        ("environment",),
        600,
        20,
        300,
    ),
    PhaseSpec(
        "model-development-replay",
        "Model-blind development corpus replay",
        phase_model_development_replay,
        ("model-inputs",),
        3 * 3600,
        400,
        1200,
    ),
    PhaseSpec(
        "model-preseal-replay",
        "Model-blind preseal receipt replay",
        phase_model_preseal_replay,
        ("model-inputs",),
        3 * 3600,
        400,
        1200,
    ),
    PhaseSpec(
        "model-strict",
        "Independent strict evaluation",
        phase_model_strict,
        ("model-inputs",),
        4 * 3600,
        900,
        9500,
        exclusive=True,
    ),
    PhaseSpec(
        "model-replays",
        "Byte-identical retraining replays",
        phase_model_replays,
        ("model-inputs",),
        16 * 3600,
        4 * 3600,
        REPLAY_MEMORY_MIB,
    ),
    PhaseSpec(
        "model-replay-strict",
        "Strict evaluation of replay a",
        phase_model_replay_strict,
        ("model-replays",),
        4 * 3600,
        900,
        4000,
        exclusive=True,
    ),
    PhaseSpec(
        "typecheck",
        "Maximum strict mypy",
        phase_typecheck,
        ("environment",),
        3600,
        120,
        600,
    ),
    PhaseSpec(
        "coverage",
        "Unit and GTK tests with 100% branch coverage",
        phase_coverage,
        ("environment",),
        2 * 3600,
        180,
        800,
        "display",
    ),
    PhaseSpec(
        "detector-gates",
        "Detector quality gates",
        phase_detector_gates,
        ("environment",),
        2 * 3600,
        600,
        400,
    ),
    PhaseSpec(
        "e2e-x11",
        "Real X11 RECORD/XTEST end-to-end",
        phase_e2e_x11,
        ("environment",),
        1800,
        120,
        500,
        "display",
    ),
    PhaseSpec(
        "e2e-tray",
        "StatusNotifierItem and DBusMenu integration",
        phase_e2e_tray,
        ("environment",),
        1800,
        60,
        200,
        "display",
    ),
    PhaseSpec(
        "build-deb",
        "Native Debian package (Nuitka)",
        phase_build_deb,
        ("environment", "model-inputs", "model-strict"),
        6 * 3600,
        600,
        4000,
    ),
    PhaseSpec(
        "verify-deb",
        "Package verifier, desktop file and Lintian",
        phase_verify_deb,
        ("build-deb",),
        1800,
        120,
        300,
    ),
    PhaseSpec(
        "e2e-native",
        "Packaged executable X11 and tray end-to-end",
        phase_e2e_native,
        ("verify-deb",),
        1800,
        120,
        600,
        "display",
    ),
    PhaseSpec(
        "release-metadata",
        "Version, changelog and model documentation",
        phase_release_metadata,
        ("environment",),
        600,
        10,
        200,
    ),
)
PHASE_BY_NAME: Final[Mapping[str, PhaseSpec]] = {spec.name: spec for spec in PHASES}

PROFILES: Final[Mapping[str, tuple[str, ...]]] = {
    "quick": (
        "environment",
        "model-inputs",
        "typecheck",
        "coverage",
        "detector-gates",
        "release-metadata",
    ),
    "app": (
        "environment",
        "model-inputs",
        "model-strict",
        "typecheck",
        "coverage",
        "detector-gates",
        "e2e-x11",
        "e2e-tray",
        "build-deb",
        "verify-deb",
        "e2e-native",
        "release-metadata",
    ),
    "release": tuple(spec.name for spec in PHASES),
}

# Runbook section 12 checklist mapped to the phases that prove each item.
CHECKLIST: Final[tuple[tuple[str, tuple[str, ...]], ...]] = (
    ("Frozen source checksums and license evidence", ("model-inputs",)),
    ("Development corpus reproduces byte for byte", ("model-development-replay",)),
    (
        "Preseal receipt is model-blind and reproducible",
        ("model-inputs", "model-preseal-replay"),
    ),
    ("Registry, manifest and test-report agree", ("model-inputs",)),
    ("Independent strict report passes every gate", ("model-strict",)),
    ("Official and replay outputs are byte-identical", ("model-replays",)),
    ("Strict typing", ("typecheck",)),
    ("100% line and branch coverage", ("coverage",)),
    ("Detector quality gates", ("detector-gates",)),
    ("X11 and tray end-to-end", ("e2e-x11", "e2e-tray")),
    ("Native Debian package verifier", ("build-deb", "verify-deb")),
    ("Packaged executable end-to-end", ("e2e-native",)),
    ("Version, changelog and model documentation", ("release-metadata",)),
)


def phase_memory_estimate(ctx: Context, name: str) -> int:
    spec = PHASE_BY_NAME[name]
    if name == "model-replays":
        # Replays run one at a time; an adopted trainer that is still running
        # counts fully because its worker pool may not have forked yet.
        return 300 + (REPLAY_MEMORY_MIB if replay_work_remaining(ctx) > 0 else 0)
    return spec.memory_mib


def phase_occupies_job_slot(ctx: Context, name: str) -> bool:
    """Adopting replays that already run elsewhere only waits; it is not work."""

    if name == "model-replays":
        return replay_launch_count(ctx) > 0
    return True


# --------------------------------------------------------------------------
# Selection, state persistence and reporting
# --------------------------------------------------------------------------


def select_phases(options: Options) -> tuple[str, ...]:
    if options.profile not in PROFILES:
        raise UsageError(f"unknown profile {options.profile!r}")
    names = list(PROFILES[options.profile])
    for group in (options.only, options.skip):
        for name in group:
            if name not in PHASE_BY_NAME:
                raise UsageError(f"unknown phase {name!r}")
    if options.only:
        names = [spec.name for spec in PHASES if spec.name in options.only]
    if options.skip:
        names = [name for name in names if name not in options.skip]
    if options.start_from:
        if options.start_from not in PHASE_BY_NAME:
            raise UsageError(f"unknown phase {options.start_from!r}")
        if options.start_from in names:
            names = names[names.index(options.start_from) :]
    if not names:
        raise UsageError("no phases selected")
    return tuple(names)


def write_json_atomic(path: Path, payload: Mapping[str, object]) -> None:
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", "utf-8")
    os.replace(temporary, path)


@dataclass
class Pipeline:
    run_dir: Path
    options: Options
    selected: tuple[str, ...]
    phases: dict[str, PhaseState]
    started_at: str
    status: str = "running"
    finished_at: str | None = None
    scheduler: dict[str, object] = field(default_factory=dict)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def to_json(self) -> dict[str, object]:
        return {
            "schema_version": STATE_SCHEMA_VERSION,
            "pipeline": {
                "status": self.status,
                "profile": self.options.profile,
                "run_dir": str(self.run_dir),
                "project_root": str(PROJECT_ROOT),
                "started_at": self.started_at,
                "finished_at": self.finished_at,
                "pid": os.getpid(),
                "argv": sys.argv,
                "selected_phases": list(self.selected),
                "jobs": self.options.jobs,
                "memory_reserve_mib": self.options.memory_reserve_mib,
                "git": {
                    "head": git_output("rev-parse", "HEAD"),
                    "branch": git_output("rev-parse", "--abbrev-ref", "HEAD"),
                    "dirty_files": len(
                        [line for line in git_output("status", "--porcelain").splitlines() if line]
                    ),
                },
                "not_covered_on_this_host": [
                    "Windows job (strict mypy on win32, Win32 unit tests, WH_KEYBOARD_LL E2E, "
                    "Inno Setup build and silent-install smoke) runs only in GitHub Actions"
                ],
            },
            "scheduler": dict(self.scheduler),
            "phases": [self.phases[name].to_json() for name in self.selected],
        }

    def save(self) -> None:
        with self.lock:
            for _attempt in range(3):
                try:
                    payload = self.to_json()
                    break
                except RuntimeError:
                    # A phase thread mutated its facts while serialising; retry.
                    time.sleep(0.05)
            else:
                return
            write_json_atomic(self.run_dir / STATE_FILE, payload)


def load_state(run_dir: Path) -> dict[str, object]:
    return load_json_object(run_dir / STATE_FILE, "pipeline state")


def phase_status_map(state: Mapping[str, object]) -> dict[str, str]:
    result: dict[str, str] = {}
    phases = state.get("phases")
    if isinstance(phases, list):
        for entry in phases:
            record = as_object(entry, "phase")
            result[as_str(record.get("name"), "name")] = as_str(record.get("status"), "status")
    return result


def checklist_rows(statuses: Mapping[str, str]) -> list[tuple[str, str]]:
    rows: list[tuple[str, str]] = []
    for label, names in CHECKLIST:
        values = [statuses.get(name, "not selected") for name in names]
        if any(value in FAILED_STATUSES for value in values):
            verdict = "FAILED"
        elif all(value == "passed" for value in values):
            verdict = "passed"
        elif all(value in {"passed", "skipped"} for value in values):
            verdict = "passed (some phases skipped)"
        elif any(value == "running" for value in values):
            verdict = "running"
        else:
            verdict = "not run"
        rows.append((label, verdict))
    rows.append(("Windows installer verifier and smoke", "not covered on this host (CI only)"))
    return rows


def render_summary_markdown(state: Mapping[str, object]) -> str:
    pipeline = as_object(state.get("pipeline"), "pipeline")
    phases_raw = state.get("phases")
    phases = (
        [as_object(item, "phase") for item in phases_raw] if isinstance(phases_raw, list) else []
    )
    statuses = phase_status_map(state)
    lines = [
        f"# KeySwitch release pipeline: {as_str(pipeline.get('status'), 'status').upper()}",
        "",
        f"- Profile: `{pipeline.get('profile')}`",
        f"- Run directory: `{pipeline.get('run_dir')}`",
        f"- Started: {pipeline.get('started_at')}; finished: {pipeline.get('finished_at')}",
        f"- Parallel jobs: {pipeline.get('jobs')}; memory reserve: "
        f"{pipeline.get('memory_reserve_mib')} MiB",
    ]
    git = pipeline.get("git")
    if isinstance(git, dict):
        lines.append(
            f"- Git: `{git.get('branch')}` at `{git.get('head')}`, "
            f"{git.get('dirty_files')} dirty paths"
        )
    lines.extend(
        [
            "",
            "## Phases",
            "",
            "| # | Phase | Status | Duration | Peak RSS | Error |",
            "| --- | --- | --- | --- | --- | --- |",
        ]
    )
    for index, phase in enumerate(phases, 1):
        error = optional_str(phase.get("error")) or ""
        lines.append(
            f"| {index} | {phase.get('name')} | {phase.get('status')} | "
            f"{format_duration(optional_number(phase.get('duration_seconds')))} | "
            f"{phase.get('observed_peak_rss_mib')} MiB | {error.replace('|', '/')} |"
        )
    lines.extend(["", "## Runbook checklist", ""])
    for label, verdict in checklist_rows(statuses):
        marker = "x" if verdict.startswith("passed") else " "
        lines.append(f"- [{marker}] {label}: {verdict}")
    lines.extend(["", "## Facts", ""])
    for phase in phases:
        facts = phase.get("facts")
        notes = phase.get("notes")
        has_facts = isinstance(facts, dict) and len(facts) > 0
        has_notes = isinstance(notes, list) and len(notes) > 0
        if not has_facts and not has_notes:
            continue
        lines.append(f"### {phase.get('name')}")
        lines.append("")
        if isinstance(notes, list):
            for note in notes:
                lines.append(f"- note: {note}")
        if isinstance(facts, dict) and len(facts) > 0:
            lines.append("```json")
            lines.append(json.dumps(facts, ensure_ascii=False, indent=2))
            lines.append("```")
        lines.append("")
    failed = [phase for phase in phases if str(phase.get("status")) in FAILED_STATUSES]
    if failed:
        lines.extend(["## Failures", ""])
        for phase in failed:
            lines.append(f"### {phase.get('name')}: {phase.get('error')}")
            lines.append("")
            lines.append(f"Log: `{phase.get('log')}`")
            lines.append("")
            tail = phase.get("log_tail")
            if isinstance(tail, list) and len(tail) > 0:
                lines.append("```text")
                lines.extend(str(item) for item in tail)
                lines.append("```")
            lines.append("")
    not_covered = pipeline.get("not_covered_on_this_host")
    if isinstance(not_covered, list):
        lines.extend(["## Not covered on this host", ""])
        lines.extend(f"- {item}" for item in not_covered)
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def write_summary(pipeline: Pipeline) -> None:
    state = pipeline.to_json()
    statuses = phase_status_map(state)
    summary: dict[str, object] = dict(state)
    summary["checklist"] = [
        {"item": label, "verdict": verdict} for label, verdict in checklist_rows(statuses)
    ]
    write_json_atomic(pipeline.run_dir / SUMMARY_JSON, summary)
    (pipeline.run_dir / SUMMARY_MARKDOWN).write_text(render_summary_markdown(state), "utf-8")


def update_latest_link(pipeline_root: Path, run_dir: Path) -> None:
    link = pipeline_root / LATEST_LINK
    try:
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(run_dir.name)
    except OSError:
        pass


def resolve_run_dir(pipeline_root: Path, argument: str) -> Path:
    if argument in {"", LATEST_LINK}:
        link = pipeline_root / LATEST_LINK
        if not link.exists():
            raise UsageError(f"no pipeline run found under {pipeline_root}")
        return link.resolve()
    candidate = Path(argument)
    if candidate.is_dir():
        return candidate.resolve()
    nested = pipeline_root / argument
    if nested.is_dir():
        return nested.resolve()
    raise UsageError(f"run directory not found: {argument}")


# --------------------------------------------------------------------------
# Scheduler
# --------------------------------------------------------------------------


class PhaseWorker(threading.Thread):
    """Runs one phase in its own thread and records the outcome."""

    def __init__(self, ctx: Context, spec: PhaseSpec, state: PhaseState, log: PhaseLog) -> None:
        super().__init__(name=f"phase-{spec.name}", daemon=True)
        self.ctx = ctx
        self.spec = spec
        self.state = state
        self.log = log
        self.aborted = False

    def run(self) -> None:
        state, log = self.state, self.log
        started = time.monotonic()
        log.write(f"=== phase {self.spec.name}: {self.spec.title} ===")
        try:
            self.spec.runner(self.ctx, log, state)
            if state.status == "running":
                state.status = "passed"
        except PhaseFailure as error:
            state.status = "failed"
            state.error = str(error)
            log.write(f"FAILED: {error}")
        except PipelineAborted:
            state.status = "aborted"
            state.error = "interrupted by a termination signal"
            self.aborted = True
            log.write("ABORTED")
        except Exception as error:  # noqa: BLE001 - recorded, not hidden
            state.status = "failed"
            state.error = f"unexpected {type(error).__name__}: {error}"
            log.write("FAILED with an unexpected exception:\n" + traceback.format_exc())
        finally:
            state.finished_at = utc_now()
            state.duration_seconds = round(time.monotonic() - started, 3)
            if state.status in FAILED_STATUSES:
                state.log_tail = log.tail()
            log.close()


def reuse_passed_phase(state: PhaseState, earlier: Mapping[str, object]) -> None:
    state.status = "passed"
    state.started_at = optional_str(earlier.get("started_at"))
    state.finished_at = optional_str(earlier.get("finished_at"))
    state.duration_seconds = optional_number(earlier.get("duration_seconds"))
    state.log = optional_str(earlier.get("log"))
    facts = earlier.get("facts")
    if isinstance(facts, dict):
        state.facts = as_object(facts, "facts")
    peak = earlier.get("observed_peak_rss_mib")
    if isinstance(peak, int) and not isinstance(peak, bool):
        state.observed_peak_rss_mib = peak
    state.notes = ["reused the passed result of the resumed run"]


def command_run(options: Options, run_dir: Path, resume: bool) -> int:
    selected = select_phases(options)
    run_dir.mkdir(parents=True, exist_ok=True)
    update_latest_link(options.pipeline_root, run_dir)
    (run_dir / PID_FILE).write_text(f"{os.getpid()}\n", "utf-8")
    previous: dict[str, dict[str, object]] = {}
    if resume and (run_dir / STATE_FILE).is_file():
        earlier = load_state(run_dir)
        raw_phases = earlier.get("phases")
        if isinstance(raw_phases, list):
            for entry in raw_phases:
                record = as_object(entry, "phase")
                previous[as_str(record.get("name"), "name")] = record

    phases = {name: PhaseState(name, PHASE_BY_NAME[name].title) for name in selected}
    pipeline = Pipeline(run_dir, options, selected, phases, utc_now())
    ctx = Context(run_dir, options, selected)

    def handle_signal(_signum: int, _frame: FrameType | None) -> None:
        ctx.request_stop()

    signal.signal(signal.SIGTERM, handle_signal)
    signal.signal(signal.SIGINT, handle_signal)

    pending: list[str] = []
    for name in selected:
        earlier_state = previous.get(name)
        if earlier_state is not None and earlier_state.get("status") == "passed":
            reuse_passed_phase(phases[name], earlier_state)
        else:
            pending.append(name)
    pipeline.save()

    running: dict[str, PhaseWorker] = {}
    blocked: dict[str, str] = {}
    pressure_events: list[dict[str, object]] = []
    failed = False
    aborted = False
    log_index = 0

    def deps_of(name: str) -> tuple[str, ...]:
        return tuple(dep for dep in PHASE_BY_NAME[name].depends if dep in selected)

    while pending or running:
        # Reap finished workers.
        for name, worker in list(running.items()):
            if worker.is_alive():
                continue
            worker.join()
            running.pop(name)
            if worker.aborted:
                aborted = True
            elif phases[name].status in FAILED_STATUSES:
                failed = True

        # Propagate blocking from failed or blocked dependencies.
        for name in list(pending):
            reasons = [
                f"{dep} {phases[dep].status}"
                for dep in deps_of(name)
                if phases[dep].status in FAILED_STATUSES or dep in blocked
            ]
            stop_now = ctx.stop_requested or aborted or (failed and options.fail_fast)
            if reasons or stop_now:
                state = phases[name]
                state.status = "skipped"
                state.notes.append(
                    "blocked by " + ", ".join(reasons) if reasons else "pipeline stopped early"
                )
                blocked[name] = state.notes[-1]
                pending.remove(name)

        # Admit ready phases within the job and memory budget.
        ready = [
            name
            for name in pending
            if all(phases[dep].status in {"passed", "skipped"} for dep in deps_of(name))
        ]
        ready.sort(key=lambda name: -PHASE_BY_NAME[name].expected_seconds)
        info = memory_info()
        available = info.get("memavailable_mib", 0)
        unrealized = 0
        for name in running:
            observed = session_rss_mib(ctx.session_ids(name))
            state = phases[name]
            state.observed_peak_rss_mib = max(state.observed_peak_rss_mib, observed)
            unrealized += max(0, phase_memory_estimate(ctx, name) - observed)
        busy_lanes = {PHASE_BY_NAME[name].lane for name in running if PHASE_BY_NAME[name].lane}
        occupied_slots = sum(1 for name in running if phase_occupies_job_slot(ctx, name))
        exclusive_running = [name for name in running if PHASE_BY_NAME[name].exclusive]
        waiting: dict[str, str] = {}
        for name in ready:
            spec = PHASE_BY_NAME[name]
            if exclusive_running:
                waiting[name] = f"exclusive phase running: {', '.join(exclusive_running)}"
                continue
            if spec.exclusive and running:
                waiting[name] = (
                    "exclusive phase waits for an idle host: "
                    + ", ".join(sorted(running))
                )
                continue
            if occupied_slots >= options.jobs:
                waiting[name] = f"job slots busy ({options.jobs})"
                continue
            if spec.lane and spec.lane in busy_lanes:
                waiting[name] = f"lane {spec.lane!r} busy"
                continue
            need = phase_memory_estimate(ctx, name)
            headroom = available - unrealized - options.memory_reserve_mib
            if running and headroom < need:
                waiting[name] = (
                    f"waiting for memory: need {need} MiB, headroom {headroom} MiB "
                    f"(available {available}, unrealized {unrealized}, "
                    f"reserve {options.memory_reserve_mib})"
                )
                continue
            pending.remove(name)
            log_index += 1
            log = PhaseLog(run_dir / "phases" / f"{log_index:02d}-{name}.log")
            state = phases[name]
            state.log = str(log.path)
            state.status = "running"
            state.started_at = utc_now()
            state.waiting_reason = None
            worker = PhaseWorker(ctx, spec, state, log)
            running[name] = worker
            worker.start()
            unrealized += need
            if phase_occupies_job_slot(ctx, name):
                occupied_slots += 1
            if spec.lane:
                busy_lanes.add(spec.lane)
            if spec.exclusive:
                exclusive_running.append(name)
        for name in pending:
            phases[name].waiting_reason = waiting.get(name)
        if running and available < options.memory_reserve_mib:
            pressure_events.append(
                {
                    "at": utc_now(),
                    "memory_available_mib": available,
                    "running": sorted(running),
                }
            )
            del pressure_events[:-50]
            print(
                f"[{utc_now()}] memory pressure: {available} MiB available below the "
                f"{options.memory_reserve_mib} MiB reserve while running {sorted(running)}",
                flush=True,
            )
        pipeline.scheduler = {
            "updated_at": utc_now(),
            "running": sorted(running),
            "waiting": waiting,
            "memory_available_mib": available,
            "memory_unrealized_mib": unrealized,
            "memory_pressure_events": list(pressure_events),
        }
        pipeline.save()
        if running:
            time.sleep(SCHEDULER_POLL_SECONDS)

    pipeline.status = "aborted" if aborted else ("failed" if failed else "passed")
    pipeline.finished_at = utc_now()
    pipeline.scheduler = {
        "updated_at": utc_now(),
        "running": [],
        "waiting": {},
        "memory_pressure_events": list(pressure_events),
    }
    pipeline.save()
    write_summary(pipeline)
    print(f"{pipeline.status.upper()}: {run_dir / SUMMARY_MARKDOWN}")
    if aborted:
        return 130
    return 1 if failed else 0


# --------------------------------------------------------------------------
# Commands
# --------------------------------------------------------------------------


def pipeline_running_elsewhere() -> list[str]:
    marker = f"{Path(__file__).name} run"
    return [line for line in process_command_lines() if marker in line]


def command_start(options: Options, run_dir: Path, resume: bool) -> int:
    running = pipeline_running_elsewhere()
    if running:
        print("another pipeline run is active; refusing to start a second one:", file=sys.stderr)
        for line in running:
            print("  " + line, file=sys.stderr)
        return 2
    run_dir.mkdir(parents=True, exist_ok=True)
    argv = [sys.executable, str(Path(__file__).resolve()), "run", *options.to_argv()]
    argv.extend(["--run-dir", str(run_dir)])
    if resume:
        argv.append("--resume-in-place")
    with (run_dir / PIPELINE_LOG).open("ab") as log_handle:
        log_handle.write(f"[{utc_now()}] $ {shlex.join(argv)}\n".encode("utf-8"))
        process = subprocess.Popen(
            argv,
            cwd=PROJECT_ROOT,
            stdin=subprocess.DEVNULL,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            env={**os.environ, "PYTHONUNBUFFERED": "1"},
        )
    update_latest_link(options.pipeline_root, run_dir)
    script = relative(Path(__file__).resolve())
    print(f"started detached pipeline pid={process.pid}")
    print(f"run_dir: {run_dir}")
    print(f"status:  python3 {script} status")
    print(f"wait:    python3 {script} wait")
    print(f"summary: {run_dir / SUMMARY_MARKDOWN} (written when the run ends)")
    return 0


def last_log_line(path_text: object) -> str:
    path = Path(path_text) if isinstance(path_text, str) else None
    if path is None or not path.is_file():
        return ""
    lines = [
        line.strip()
        for line in path.read_bytes().decode("utf-8", "replace").splitlines()
        if line.strip()
    ]
    return lines[-1] if lines else ""


def elapsed_since(started_raw: object) -> float | None:
    if not isinstance(started_raw, str):
        return None
    started = dt.datetime.strptime(started_raw, "%Y-%m-%dT%H:%M:%SZ").replace(
        tzinfo=dt.timezone.utc
    )
    return (dt.datetime.now(dt.timezone.utc) - started).total_seconds()


def command_status(run_dir: Path, as_json: bool) -> int:
    state = load_state(run_dir)
    if as_json:
        print(json.dumps(state, ensure_ascii=False, indent=2))
    else:
        pipeline = as_object(state.get("pipeline"), "pipeline")
        print(f"run: {run_dir}")
        print(
            f"status: {pipeline.get('status')} (profile {pipeline.get('profile')}, "
            f"started {pipeline.get('started_at')}, jobs {pipeline.get('jobs')})"
        )
        scheduler = state.get("scheduler")
        if isinstance(scheduler, dict) and pipeline.get("status") == "running":
            print(
                f"memory: available {scheduler.get('memory_available_mib')} MiB, "
                f"unrealized {scheduler.get('memory_unrealized_mib')} MiB"
            )
        phases = state.get("phases")
        if isinstance(phases, list):
            for entry in phases:
                record = as_object(entry, "phase")
                status = str(record.get("status"))
                shown = format_duration(optional_number(record.get("duration_seconds")))
                if status == "running":
                    shown = format_duration(elapsed_since(record.get("started_at"))) + " so far"
                line = f"  {status:<8} {shown:>14}  {record.get('name')}"
                if status == "running":
                    tail = last_log_line(record.get("log"))
                    if tail:
                        line += f"\n           last: {tail[:160]}"
                waiting = optional_str(record.get("waiting_reason"))
                if status == "pending" and waiting:
                    line += f"\n           {waiting}"
                error = optional_str(record.get("error"))
                if error:
                    line += f"\n           error: {error}"
                print(line)
        summary = run_dir / SUMMARY_MARKDOWN
        if summary.is_file():
            print(f"summary: {summary}")
    pipeline_status = lookup(state, "pipeline", "status")
    if pipeline_status == "passed":
        return 0
    if pipeline_status == "running":
        return 3
    return 1


def command_wait(run_dir: Path, poll_seconds: int) -> int:
    while True:
        state = load_state(run_dir)
        status = lookup(state, "pipeline", "status")
        if status != "running":
            return command_status(run_dir, False)
        pid_file = run_dir / PID_FILE
        pid_text = pid_file.read_text("utf-8").strip() if pid_file.is_file() else ""
        if pid_text.isdigit() and not Path(f"/proc/{pid_text}").exists():
            print(
                f"pipeline process {pid_text} is gone but state.json still says running",
                file=sys.stderr,
            )
            return 1
        time.sleep(poll_seconds)


def command_phases() -> int:
    print(f"{'phase':<26} {'timeout':>8} {'memory':>9}  {'lane':<8} {'profiles':<18} depends")
    for spec in PHASES:
        profiles = ",".join(name for name, members in PROFILES.items() if spec.name in members)
        print(
            f"{spec.name:<26} {format_duration(spec.timeout_seconds):>8} "
            f"{spec.memory_mib:>6} MiB  {spec.lane or '-':<8} {profiles:<18} "
            f"{', '.join(spec.depends) or '-'}"
        )
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    commands = parser.add_subparsers(dest="command", required=True)
    default_jobs = max(1, min(4, available_cpus() // 3))

    def add_run_options(target: argparse.ArgumentParser) -> None:
        target.add_argument("--profile", choices=sorted(PROFILES), default="app")
        target.add_argument("--only", default="", help="comma-separated phases to run")
        target.add_argument("--skip", default="", help="comma-separated phases to skip")
        target.add_argument("--from", dest="start_from", default="", help="first phase to run")
        target.add_argument("--replays", type=int, default=2, help="retraining replays (0-2)")
        target.add_argument(
            "--replay-dir",
            default="",
            help="directory with replay subdirectories a/ and b/; finished or running "
            "replays there are adopted instead of started",
        )
        target.add_argument(
            "--replay-strict",
            action="store_true",
            help="also run the strict evaluator on replay a",
        )
        target.add_argument(
            "--strict-report",
            default="",
            help="existing strict report to reuse when it matches the bundled artifact",
        )
        target.add_argument("--workers", type=int, default=0, help="trainer --workers for replays")
        target.add_argument(
            "--jobs",
            type=int,
            default=default_jobs,
            help=f"maximum concurrently running phases (default {default_jobs})",
        )
        target.add_argument(
            "--memory-reserve-mib",
            type=int,
            default=DEFAULT_MEMORY_RESERVE_MIB,
            help="RAM that must stay available after admitting a phase",
        )
        target.add_argument("--timeout-scale", type=float, default=1.0)
        target.add_argument(
            "--fail-fast",
            action="store_true",
            help="stop admitting phases after the first failure (default: keep going)",
        )
        target.add_argument("--pipeline-root", default=str(DEFAULT_PIPELINE_ROOT))
        target.add_argument("--run-dir", default="", help="explicit run directory")
        target.add_argument(
            "--resume",
            default="",
            help="run directory whose passed phases are reused; the run continues in place",
        )
        target.add_argument("--resume-in-place", action="store_true", help=argparse.SUPPRESS)

    add_run_options(commands.add_parser("run", help="run the pipeline in the foreground"))
    add_run_options(
        commands.add_parser("start", help="run the pipeline detached from the terminal")
    )
    status = commands.add_parser("status", help="show the state of a run")
    status.add_argument("run", nargs="?", default=LATEST_LINK)
    status.add_argument("--json", action="store_true")
    status.add_argument("--pipeline-root", default=str(DEFAULT_PIPELINE_ROOT))
    wait = commands.add_parser("wait", help="block until a run finishes")
    wait.add_argument("run", nargs="?", default=LATEST_LINK)
    wait.add_argument("--poll", type=int, default=60)
    wait.add_argument("--pipeline-root", default=str(DEFAULT_PIPELINE_ROOT))
    commands.add_parser("phases", help="list phases, budgets and profiles")
    return parser


def split_names(value: str) -> tuple[str, ...]:
    return tuple(item.strip() for item in value.split(",") if item.strip())


def options_from(arguments: argparse.Namespace) -> Options:
    replays = int(arguments.replays)
    if not 0 <= replays <= 2:
        raise UsageError("--replays must be 0, 1 or 2")
    jobs = int(arguments.jobs)
    if jobs < 1:
        raise UsageError("--jobs must be at least 1")
    replay_dir = str(arguments.replay_dir)
    strict_report = str(arguments.strict_report)
    return Options(
        profile=str(arguments.profile),
        only=split_names(str(arguments.only)),
        skip=split_names(str(arguments.skip)),
        start_from=str(arguments.start_from),
        replays=replays,
        replay_dir=Path(replay_dir).resolve() if replay_dir else None,
        replay_strict=bool(arguments.replay_strict),
        strict_report=Path(strict_report).resolve() if strict_report else None,
        workers=int(arguments.workers),
        jobs=jobs,
        memory_reserve_mib=max(0, int(arguments.memory_reserve_mib)),
        timeout_scale=float(arguments.timeout_scale),
        fail_fast=bool(arguments.fail_fast),
        pipeline_root=Path(str(arguments.pipeline_root)).resolve(),
    )


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    arguments = parser.parse_args(list(argv) if argv is not None else None)
    command = str(arguments.command)
    try:
        if command == "phases":
            return command_phases()
        if command in {"status", "wait"}:
            pipeline_root = Path(str(arguments.pipeline_root)).resolve()
            run_dir = resolve_run_dir(pipeline_root, str(arguments.run))
            if command == "status":
                return command_status(run_dir, bool(arguments.json))
            return command_wait(run_dir, max(5, int(arguments.poll)))
        options = options_from(arguments)
        select_phases(options)
        resume_argument = str(arguments.resume)
        run_dir_argument = str(arguments.run_dir)
        resume = bool(resume_argument) or bool(arguments.resume_in_place)
        if resume_argument:
            run_dir = resolve_run_dir(options.pipeline_root, resume_argument)
        elif run_dir_argument:
            run_dir = Path(run_dir_argument).resolve()
        else:
            run_dir = options.pipeline_root / run_identifier(options.profile)
        if command == "start":
            return command_start(options, run_dir, resume)
        return command_run(options, run_dir, resume)
    except UsageError as error:
        parser.error(str(error))
    except PhaseFailure as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
