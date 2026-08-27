"""Single-instance coordination for the Windows frontend."""

from __future__ import annotations

import sys
from typing import Protocol

from .windows_system import WindowsSystemError


class WindowsInstanceAPI(Protocol):
    def acquire(self) -> bool: ...

    def activate_existing(self) -> bool: ...

    def close(self) -> None: ...


def _running_on_windows() -> bool:
    return sys.platform == "win32"


def _native_api() -> WindowsInstanceAPI:
    if not _running_on_windows():
        raise WindowsSystemError("Single-instance Win32 доступен только в Windows")
    from .windows_instance_native import CtypesWindowsInstanceAPI

    return CtypesWindowsInstanceAPI()


class WindowsSingleInstance:
    def __init__(self, api: WindowsInstanceAPI | None = None) -> None:
        self._api = api or _native_api()
        self._closed = False

    def acquire(self) -> bool:
        return False if self._closed else self._api.acquire()

    def activate_existing(self) -> bool:
        return False if self._closed else self._api.activate_existing()

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._api.close()
