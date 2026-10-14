"""How KeySwitch calls the Win32 API through ctypes on every Windows architecture.

User32 and Kernel32 export `__stdcall` functions and call a hook procedure the same way. On 32-bit x86
Windows only `ctypes.WinDLL` and `ctypes.WINFUNCTYPE` follow that convention (`ctypes.CDLL` and
`CFUNCTYPE` are cdecl there); on x64 and Arm64 Windows there is one convention, and they are the same as
`CDLL` and `CFUNCTYPE`. Both are looked up when called: elsewhere ctypes has neither, and the cdecl ones
serve the mocks and the type checks of the Windows code.
"""

from __future__ import annotations

import ctypes
from collections.abc import Callable
from typing import cast

from _ctypes import CFuncPtr

_HookPrototypeFactory = Callable[
    [type[ctypes.c_ssize_t], type[ctypes.c_int], type[ctypes.c_size_t], type[ctypes.c_ssize_t]],
    type[CFuncPtr],
]


def stdcall_library() -> type[ctypes.CDLL]:
    """The loader of a Win32 system library such as user32.dll."""

    return cast(type[ctypes.CDLL], getattr(ctypes, "WinDLL", ctypes.CDLL))


def hook_prototype() -> type[CFuncPtr]:
    """The prototype of a low-level hook procedure: `LRESULT CALLBACK (int, WPARAM, LPARAM)`."""

    factory = cast(_HookPrototypeFactory, getattr(ctypes, "WINFUNCTYPE", ctypes.CFUNCTYPE))
    return factory(ctypes.c_ssize_t, ctypes.c_int, ctypes.c_size_t, ctypes.c_ssize_t)
