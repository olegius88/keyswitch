"""Testable Windows profile integration and application discovery."""

from __future__ import annotations

import ntpath
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol


AUTOSTART_VALUE_NAME = "KeySwitch"


class WindowsSystemError(RuntimeError):
    pass


class WindowsRegistry(Protocol):
    def read_autostart(self, name: str) -> str | None: ...

    def write_autostart(self, name: str, command: str) -> None: ...

    def delete_autostart(self, name: str) -> None: ...

    def application_paths(self) -> tuple[tuple[str, str], ...]: ...


@dataclass(frozen=True)
class WindowsApplication:
    name: str
    identifier: str
    executable: str


def _running_on_windows() -> bool:
    return sys.platform == "win32"


def _default_registry() -> WindowsRegistry:
    if not _running_on_windows():
        raise WindowsSystemError("Реестр Windows доступен только в Windows")
    from .windows_registry import NativeWindowsRegistry

    return NativeWindowsRegistry()


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
    ) -> None:
        self._registry = registry or _default_registry()
        self._command = command

    def enabled(self) -> bool:
        return bool(self._registry.read_autostart(AUTOSTART_VALUE_NAME))

    def set_enabled(self, enabled: bool, *, start_hidden: bool = True) -> None:
        if enabled:
            command = self._command or windows_launcher_command(
                start_hidden=start_hidden
            )
            self._registry.write_autostart(AUTOSTART_VALUE_NAME, command)
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
