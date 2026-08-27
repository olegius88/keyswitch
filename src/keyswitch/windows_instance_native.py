"""Native named-mutex and existing-window adapter for Windows."""

from __future__ import annotations

import ctypes

from .windows_system import WindowsSystemError


MUTEX_NAME = r"Local\io.github.olegius88.KeySwitch"
WINDOW_TITLE = "KeySwitch"
ERROR_ALREADY_EXISTS = 183
SW_RESTORE = 9


class CtypesWindowsInstanceAPI:
    def __init__(self) -> None:
        self.kernel32 = ctypes.CDLL("kernel32.dll", use_last_error=True)
        self.user32 = ctypes.CDLL("user32.dll", use_last_error=True)
        self._handle: int | None = None
        self._declare()

    def _declare(self) -> None:
        self.kernel32.CreateMutexW.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_wchar_p,
        ]
        self.kernel32.CreateMutexW.restype = ctypes.c_void_p
        self.kernel32.GetLastError.argtypes = []
        self.kernel32.GetLastError.restype = ctypes.c_ulong
        self.kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
        self.kernel32.CloseHandle.restype = ctypes.c_int
        self.user32.FindWindowW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]
        self.user32.FindWindowW.restype = ctypes.c_void_p
        self.user32.ShowWindow.argtypes = [ctypes.c_void_p, ctypes.c_int]
        self.user32.ShowWindow.restype = ctypes.c_int
        self.user32.SetForegroundWindow.argtypes = [ctypes.c_void_p]
        self.user32.SetForegroundWindow.restype = ctypes.c_int

    def acquire(self) -> bool:
        if self._handle is not None:
            return True
        handle = self.kernel32.CreateMutexW(None, 0, MUTEX_NAME)
        if not handle:
            code = int(self.kernel32.GetLastError())
            raise WindowsSystemError(
                f"Не удалось создать single-instance mutex (Win32 error {code})"
            )
        self._handle = int(handle)
        return int(self.kernel32.GetLastError()) != ERROR_ALREADY_EXISTS

    def activate_existing(self) -> bool:
        window = self.user32.FindWindowW(None, WINDOW_TITLE)
        if not window:
            return False
        self.user32.ShowWindow(window, SW_RESTORE)
        return bool(self.user32.SetForegroundWindow(window))

    def close(self) -> None:
        handle, self._handle = self._handle, None
        if handle is not None:
            self.kernel32.CloseHandle(handle)
