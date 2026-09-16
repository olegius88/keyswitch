"""Testable Windows profile integration and application discovery."""

from __future__ import annotations

import ntpath
import shlex
import subprocess
import sys
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol


AUTOSTART_VALUE_NAME = "KeySwitch"

DirectoryOpener = Callable[[list[str]], None]


class WindowsSystemError(RuntimeError):
    pass


def _spawn_explorer(arguments: list[str]) -> None:
    subprocess.Popen(arguments, close_fds=True)


def open_directory(
    path: Path,
    *,
    spawn: DirectoryOpener = _spawn_explorer,
) -> None:
    """Show a folder in Explorer.

    Explorer reports success with a non-zero exit code, so the process is only
    started and never waited for; a missing folder is reported here instead.
    """

    if not path.is_dir():
        raise WindowsSystemError(f"Каталог не найден: {path}")
    spawn(["explorer", str(path)])


class WindowsRegistry(Protocol):
    def read_autostart(self, name: str) -> str | None: ...

    def write_autostart(self, name: str, command: str) -> None: ...

    def delete_autostart(self, name: str) -> None: ...

    def read_startup_approval(self, name: str) -> bytes | None: ...

    def clear_startup_approval(self, name: str) -> None: ...

    def application_paths(self) -> tuple[tuple[str, str], ...]: ...


@dataclass(frozen=True)
class WindowsApplication:
    name: str
    identifier: str
    executable: str


@dataclass(frozen=True)
class AutostartStatus:
    """What the user actually gets at the next logon, and why.

    ``requested`` is the KeySwitch setting, ``command`` the value under ``Run``. Windows
    keeps its own approval byte per value under ``Explorer\\StartupApproved\\Run``: Task
    Manager's Startup tab and Settings write it, and a value marked disabled there is
    skipped at logon however often it is rewritten. ``effective`` answers the user's
    question - will KeySwitch start with Windows.
    """

    command: str | None
    blocked_by_windows: bool
    target_missing: bool

    @property
    def effective(self) -> bool:
        return bool(self.command) and not self.blocked_by_windows and not self.target_missing

    def as_dict(self) -> dict[str, object]:
        return {"command": self.command, "blocked_by_windows": self.blocked_by_windows,
                "target_missing": self.target_missing, "effective": self.effective}


def _running_on_windows() -> bool:
    return sys.platform == "win32"


def _default_registry() -> WindowsRegistry:
    if not _running_on_windows():
        raise WindowsSystemError("Реестр Windows доступен только в Windows")
    from .windows_registry import NativeWindowsRegistry

    return NativeWindowsRegistry()


def _executable_exists(program: str) -> bool:
    # ``os.path.isfile`` behind ``Path.is_file`` answers False for a path the
    # system cannot evaluate at all - too long, or carrying a null - so a value
    # left in the registry by another install never raises here.
    return Path(program).is_file()


def windows_launcher_command(
    *,
    start_hidden: bool = True,
    executable: Path | None = None,
) -> str:
    """Return a correctly quoted per-user startup command."""

    program = executable or Path(sys.executable)
    if program.stem.casefold() == "keyswitch":
        arguments = [str(program)]
    else:
        pythonw = program.with_name("pythonw.exe")
        interpreter = pythonw if pythonw.is_file() else program
        arguments = [str(interpreter), "-m", "keyswitch"]
    if start_hidden:
        arguments.append("--hidden")
    return subprocess.list2cmdline(arguments)


class WindowsAutostartManager:
    """Manage the current user's ``Run`` value through a narrow adapter."""

    def __init__(
        self,
        registry: WindowsRegistry | None = None,
        *,
        command: str | None = None,
        exists: Callable[[str], bool] | None = None,
    ) -> None:
        self._registry = registry or _default_registry()
        self._command = command
        # Injected so the behaviour can be exercised off Windows, where a Windows path
        # never resolves and every command would look like a missing target.
        self._exists = exists or _executable_exists

    def enabled(self) -> bool:
        """True only when the next logon really starts KeySwitch."""

        return self.status().effective

    def status(self) -> AutostartStatus:
        command = self._registry.read_autostart(AUTOSTART_VALUE_NAME)
        approval = self._registry.read_startup_approval(AUTOSTART_VALUE_NAME)
        # The first byte carries the state; 0x02 and 0x06 mean enabled, anything else
        # (0x03 from Task Manager, 0x01 from older builds) means the value is skipped.
        blocked = approval is not None and (not approval or approval[0] not in (0x02, 0x06))
        missing = bool(command) and not self._target_exists(str(command))
        return AutostartStatus(command, blocked, missing)

    def _target_exists(self, command: str) -> bool:
        program = next(iter(shlex.split(command, posix=False)), "").strip('"')
        return bool(program) and self._exists(program)

    def set_enabled(self, enabled: bool, *, start_hidden: bool = True, clear_windows_block: bool = False) -> None:
        """Write or remove the value; ``clear_windows_block`` also lifts a Windows block.

        The startup sync at every launch must not fight a choice the user made in Windows,
        so only an explicit toggle in the KeySwitch interface clears the approval record.
        """

        if enabled:
            command = self._command or windows_launcher_command(
                start_hidden=start_hidden
            )
            self._registry.write_autostart(AUTOSTART_VALUE_NAME, command)
            if clear_windows_block:
                self._registry.clear_startup_approval(AUTOSTART_VALUE_NAME)
        else:
            self._registry.delete_autostart(AUTOSTART_VALUE_NAME)


class WindowsApplicationCatalog:
    """List registered executables in the form consumed by exclusions."""

    def __init__(self, registry: WindowsRegistry | None = None) -> None:
        self._registry = registry or _default_registry()

    def installed(self) -> tuple[WindowsApplication, ...]:
        applications: dict[str, WindowsApplication] = {}
        for registered_name, executable in self._registry.application_paths():
            clean_executable = clean_windows_executable(executable)
            executable_name = ntpath.basename(clean_executable)
            registered_basename = ntpath.basename(registered_name.strip())
            identifier_source = executable_name or registered_basename
            identifier = ntpath.splitext(identifier_source)[0].casefold()
            if not identifier:
                continue
            display_name = ntpath.splitext(registered_basename)[0] or identifier
            applications.setdefault(
                identifier,
                WindowsApplication(display_name, identifier, clean_executable),
            )
        return tuple(
            sorted(
                applications.values(),
                key=lambda application: application.name.casefold(),
            )
        )

    @staticmethod
    def from_executable(executable: str) -> WindowsApplication | None:
        clean_executable = clean_windows_executable(executable)
        basename = ntpath.basename(clean_executable)
        identifier = ntpath.splitext(basename)[0].casefold()
        if not identifier:
            return None
        return WindowsApplication(
            ntpath.splitext(basename)[0],
            identifier,
            clean_executable,
        )


def clean_windows_executable(value: str) -> str:
    """Remove quotes and a DisplayIcon index from an executable path."""

    text = value.strip()
    if text.startswith('"'):
        closing_quote = text.find('"', 1)
        if closing_quote > 0:
            return text[1:closing_quote]
    candidate, separator, icon_index = text.rpartition(",")
    if separator and icon_index.strip().lstrip("-").isdigit():
        text = candidate
    return text.strip().strip('"')
