"""What the settings window needs from the system it is running on.

The window itself is the same everywhere: the same pages, the same settings, the
same wording. Only a handful of things differ - which keyboard backend to build,
where autostart is kept, how the installed programs are listed, how a folder is
opened and what the alert sound is - and they are named here so each platform
can supply its own without the window knowing which one it got.
"""

from __future__ import annotations

from pathlib import Path
from typing import Protocol

from .backend import InputBackend, ScreenAnchor
from .system_model import Application, AutostartStatus
from .tray_model import TrayActions, TrayController


class PromptBackend(InputBackend, Protocol):
    """What the learning prompt needs beyond watching the keyboard.

    The prompt appears over the text the user is typing and must not take the
    focus away from it. Windows needs to be told both things explicitly; a
    system that does neither answers that it did nothing.
    """

    def input_anchor(self) -> ScreenAnchor | None: ...

    def restore_window(self, window: int | None) -> bool: ...

    def keep_window_inactive(self, window: int) -> bool: ...


class AutostartManager(Protocol):
    def enabled(self) -> bool: ...

    def status(self) -> AutostartStatus: ...

    def set_enabled(
        self,
        enabled: bool,
        *,
        start_hidden: bool = True,
        override_system_block: bool = False,
    ) -> None: ...


class ApplicationCatalog(Protocol):
    def installed(self) -> tuple[Application, ...]: ...

    def from_executable(self, executable: str) -> Application | None: ...


class DesktopServices(Protocol):
    """One platform's answers, handed to the window when it is built."""

    @property
    def backend_label(self) -> str: ...

    def backend(self) -> PromptBackend: ...

    def autostart(self) -> AutostartManager: ...

    def catalog(self) -> ApplicationCatalog: ...

    def tray(self, actions: TrayActions) -> TrayController: ...

    def open_directory(self, path: Path) -> None: ...

    def beep(self) -> None: ...
