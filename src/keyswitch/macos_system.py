"""Autostart and the list of installed programs, as macOS keeps them.

Nothing here talks to a framework: a launch agent is a file, and the installed
programs are directories, so all of it is decided in plain Python and verified
on any host. The calls that do need macOS - opening a folder, the alert sound -
live in :mod:`keyswitch.macos_objc`.
"""

from __future__ import annotations

import plistlib
import shlex
import sys
from collections.abc import Callable, Iterable, Sequence
from pathlib import Path

from .system import APP_ID
from .system_model import Application, AutostartStatus

# launchd reads every property list in this directory when the user logs in.
LAUNCH_AGENTS_DIRECTORY = "Library/LaunchAgents"
AUTOSTART_LABEL = APP_ID
LABEL_KEY = "Label"
ARGUMENTS_KEY = "ProgramArguments"
RUN_AT_LOAD_KEY = "RunAtLoad"
HIDDEN_ARGUMENT = "--hidden"
MODULE_ARGUMENTS = ("-m", "keyswitch")
INSTALLED_NAME = "keyswitch"
BUNDLE_SUFFIX = ".app"

# Where programs live on macOS. The first two are the system's own, the third is
# a user's private one; all three are read, none is required to exist.
APPLICATION_DIRECTORIES = ("/Applications", "/System/Applications", "~/Applications")
# Apple keeps its smaller programs one level down, and installers use the same
# place, so the search goes one directory deeper and no further.
NESTED_DIRECTORIES = ("Utilities",)


class MacSystemError(RuntimeError):
    """The system refused something the settings window asked for."""


def launch_agent_path(home: Path | None = None) -> Path:
    base = home if home is not None else Path.home()
    return base / LAUNCH_AGENTS_DIRECTORY / f"{AUTOSTART_LABEL}.plist"


def macos_launcher_arguments(
    executable: Path | None = None,
    *,
    start_hidden: bool = True,
) -> list[str]:
    """What launchd should run: the installed program, or this interpreter."""

    program = executable if executable is not None else Path(sys.executable)
    if program.stem.casefold() == INSTALLED_NAME:
        arguments = [str(program)]
    else:
        arguments = [str(program), *MODULE_ARGUMENTS]
    if start_hidden:
        arguments.append(HIDDEN_ARGUMENT)
    return arguments


class MacAutostartManager:
    """Manage the launch agent that starts KeySwitch when the user logs in."""

    def __init__(
        self,
        path: Path | None = None,
        *,
        arguments: Sequence[str] | None = None,
        exists: Callable[[str], bool] | None = None,
    ) -> None:
        self._path = path if path is not None else launch_agent_path()
        self._arguments = list(arguments) if arguments is not None else None
        # Injected so the behaviour can be exercised off macOS, where a macOS
        # path never resolves and every agent would look like a missing target.
        self._exists = exists if exists is not None else _program_exists

    @property
    def path(self) -> Path:
        return self._path

    def enabled(self) -> bool:
        return self.status().effective

    def status(self) -> AutostartStatus:
        arguments = self._registered_arguments()
        if not arguments:
            return AutostartStatus(None, False, False)
        return AutostartStatus(
            shlex.join(arguments), False, not self._exists(arguments[0]))

    def _registered_arguments(self) -> list[str]:
        """What the agent will run, or nothing when it will not run at all.

        An agent whose ``RunAtLoad`` is off stays on disk and starts nothing, so
        it is reported as no autostart rather than as an autostart that works.
        """

        try:
            with self._path.open("rb") as handle:
                document = plistlib.load(handle)
        except (OSError, plistlib.InvalidFileException, ValueError):
            return []
        if not isinstance(document, dict) or not document.get(RUN_AT_LOAD_KEY):
            return []
        arguments = document.get(ARGUMENTS_KEY)
        if not isinstance(arguments, list) or not arguments:
            return []
        return [str(item) for item in arguments]

    def set_enabled(
        self,
        enabled: bool,
        *,
        start_hidden: bool = True,
        override_system_block: bool = False,
    ) -> None:
        """Write or remove the agent.

        ``override_system_block`` has nothing to lift here: macOS keeps no
        separate refusal beside the agent the way Windows keeps one beside its
        startup entry. Removing the file is the whole of turning it off.
        """

        if not enabled:
            self._path.unlink(missing_ok=True)
            return
        arguments = self._arguments or macos_launcher_arguments(start_hidden=start_hidden)
        document = {
            LABEL_KEY: AUTOSTART_LABEL,
            ARGUMENTS_KEY: list(arguments),
            RUN_AT_LOAD_KEY: True,
        }
        try:
            self._path.parent.mkdir(parents=True, exist_ok=True)
            with self._path.open("wb") as handle:
                plistlib.dump(document, handle)
        except OSError as error:
            raise MacSystemError(f"не удалось записать {self._path}: {error}") from error


def _program_exists(program: str) -> bool:
    return Path(program).exists()


class MacApplicationCatalog:
    """List installed programs in the form the exclusion list consumes."""

    def __init__(self, directories: Iterable[str | Path] | None = None) -> None:
        sources = directories if directories is not None else APPLICATION_DIRECTORIES
        self._directories = [Path(item).expanduser() for item in sources]

    def installed(self) -> tuple[Application, ...]:
        applications: dict[str, Application] = {}
        for directory in self._directories:
            for bundle in self._bundles(directory):
                application = self.from_executable(str(bundle))
                if application is not None:
                    applications.setdefault(application.identifier, application)
        return tuple(sorted(applications.values(),
                            key=lambda application: application.name.casefold()))

    def _bundles(self, directory: Path) -> list[Path]:
        found: list[Path] = []
        for place in (directory, *(directory / name for name in NESTED_DIRECTORIES)):
            try:
                entries = sorted(place.iterdir())
            except OSError:
                # A directory that is absent or unreadable is simply not a source.
                continue
            found.extend(entry for entry in entries if entry.name.endswith(BUNDLE_SUFFIX))
        return found

    @staticmethod
    def from_executable(executable: str) -> Application | None:
        """Name a program by its bundle.

        The identifier is the bundle's name folded, because that is what the
        engine is told the frontmost program is called.
        """

        path = Path(executable.strip())
        name = path.name[: -len(BUNDLE_SUFFIX)] if path.name.endswith(BUNDLE_SUFFIX) else path.name
        identifier = name.casefold()
        if not identifier:
            return None
        return Application(name, identifier, str(path))
