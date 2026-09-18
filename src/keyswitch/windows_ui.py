"""The Windows side of the settings window: which services it is given.

The window itself lives in :mod:`keyswitch.desktop_ui` and is the same on every
platform. This module supplies the Windows answers and keeps the name the
Windows build and its end-to-end test already use.
"""

from __future__ import annotations

import winsound
from pathlib import Path

from .desktop_services import ApplicationCatalog, AutostartManager, PromptBackend
from .desktop_ui import DesktopApplication, PAGE_NAMES as PAGE_NAMES, run_application

# The name the Windows end-to-end test has always used for the window.
WindowsApplication = DesktopApplication
from .tray_model import TrayActions, TrayController
from .windows_backend import WindowsBackend
from .windows_system import (
    WindowsApplicationCatalog,
    WindowsAutostartManager,
    open_directory,
)
from .windows_tray import WindowsTray

BACKEND_LABEL = "Win32 hook + SendInput"


class WindowsServices:
    """Everything the settings window needs, as Windows provides it."""

    @property
    def backend_label(self) -> str:
        return BACKEND_LABEL

    def backend(self) -> PromptBackend:
        return WindowsBackend()

    def autostart(self) -> AutostartManager:
        return WindowsAutostartManager()

    def catalog(self) -> ApplicationCatalog:
        return WindowsApplicationCatalog()

    def tray(self, actions: TrayActions) -> TrayController:
        return WindowsTray(actions)

    def open_directory(self, path: Path) -> None:
        open_directory(path)

    def beep(self) -> None:
        winsound.MessageBeep(winsound.MB_OK)


def run_windows_application(
    *,
    hidden: bool,
    no_engine: bool,
    quit_after_ms: int | None = None,
) -> int:
    return run_application(
        WindowsServices(), hidden=hidden, no_engine=no_engine, quit_after_ms=quit_after_ms)
