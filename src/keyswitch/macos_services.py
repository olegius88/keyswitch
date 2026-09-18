"""The macOS side of the settings window: which services it is given.

The window itself lives in :mod:`keyswitch.desktop_ui` and is the same on every
platform; this module supplies the macOS answers.
"""

from __future__ import annotations

from pathlib import Path

from .desktop_services import ApplicationCatalog, AutostartManager, PromptBackend
from .macos_backend import MacBackend
from .macos_system import MacApplicationCatalog, MacAutostartManager, MacSystemError
from .tray_model import TrayActions, TrayController

BACKEND_LABEL = "CGEventTap + CGEventPost"


class MacServices:
    """Everything the settings window needs, as macOS provides it."""

    def __init__(self, backend: PromptBackend | None = None) -> None:
        # The engine already holds a backend when the window opens from the menu
        # bar; building a second one would put two taps on the same keyboard.
        self._backend = backend

    @property
    def backend_label(self) -> str:
        return BACKEND_LABEL

    def backend(self) -> PromptBackend:
        if self._backend is None:
            self._backend = MacBackend()
        return self._backend

    def autostart(self) -> AutostartManager:
        return MacAutostartManager()

    def catalog(self) -> ApplicationCatalog:
        return MacApplicationCatalog()

    def tray(self, actions: TrayActions) -> TrayController:
        from .macos_tray_native import StatusItemAdapter

        return TrayController(actions, StatusItemAdapter())

    def open_directory(self, path: Path) -> None:
        from .macos_objc import open_path

        if not open_path(str(path)):
            raise MacSystemError(f"не удалось открыть {path}")

    def beep(self) -> None:
        from .macos_objc import beep

        beep()
