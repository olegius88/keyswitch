"""How KeySwitch calls the Win32 API through ctypes on every Windows architecture.

User32 and Kernel32 export `__stdcall` functions and call a hook procedure the same way. On 32-bit x86
Windows only `ctypes.WinDLL` and `ctypes.WINFUNCTYPE` follow that convention (`ctypes.CDLL` and
`CFUNCTYPE` are cdecl there); on x64 and Arm64 Windows there is one convention, and they are the same as
`CDLL` and `CFUNCTYPE`. Both are looked up when called: elsewhere ctypes has neither, and the cdecl ones
serve the mocks and the type checks of the Windows code.
"""

from __future__ import annotations

import ctypes
from collections.abc import Callable, Mapping
from typing import Final, cast

from _ctypes import FUNCFLAG_PYTHONAPI, CFuncPtr

_HookPrototypeFactory = Callable[
    [type[ctypes.c_ssize_t], type[ctypes.c_int], type[ctypes.c_size_t], type[ctypes.c_ssize_t]],
    type[CFuncPtr],
]

# The calls the keyboard hook makes at every key, by library: each reads state of the system and
# returns. Only such a call may keep the GIL. One that sends a message, waits on another thread or
# makes the system call the hook (SendInput, CallNextHookEx, GetMessageW) would hold the GIL the hook
# thread is waiting for.
GIL_HOLDING_CALLS: Final[Mapping[str, tuple[str, ...]]] = {
    "user32.dll": (
        "GetAsyncKeyState",
        "GetForegroundWindow",
        "GetGUIThreadInfo",
        "GetKeyboardLayout",
        "GetWindowThreadProcessId",
        "ToUnicodeEx",
    ),
    "kernel32.dll": ("GetTickCount",),
}


def stdcall_library() -> type[ctypes.CDLL]:
    """The loader of a Win32 system library such as user32.dll."""

    return cast(type[ctypes.CDLL], getattr(ctypes, "WinDLL", ctypes.CDLL))


def gil_holding_library() -> type[ctypes.CDLL]:
    """The loader of a system library whose functions keep the GIL while they run.

    Every other foreign call releases the GIL and takes it back on return. On the keyboard hook's
    path that wait holds every key of the system: a busy engine thread kept the hook 14-50 ms per
    call, up to 52 ms per key, in the Windows VM under load (27.10.2026). Short calls that neither
    block nor reach the hook again keep it instead, as `ctypes.PyDLL` does, in the stdcall convention
    of `stdcall_library`.
    """

    base = stdcall_library()
    library = type("GilHoldingLibrary", (base,), {"_func_flags_": base._func_flags_ | FUNCFLAG_PYTHONAPI})
    return cast(type[ctypes.CDLL], library)


def keep_gil_in_short_calls(library: ctypes.CDLL, name: str) -> None:
    """Rebind the GIL_HOLDING_CALLS of ``library``, loaded as ``name``, to keep the GIL.

    Their types are declared on ``library`` first and carried over; every other function of it
    still releases the GIL.
    """

    holding = gil_holding_library()(name, use_last_error=True)
    for function_name in GIL_HOLDING_CALLS[name]:
        declared = getattr(library, function_name)
        function = getattr(holding, function_name)
        function.argtypes = declared.argtypes
        function.restype = declared.restype
        setattr(library, function_name, function)


def hook_prototype() -> type[CFuncPtr]:
    """The prototype of a low-level hook procedure: `LRESULT CALLBACK (int, WPARAM, LPARAM)`."""

    factory = cast(_HookPrototypeFactory, getattr(ctypes, "WINFUNCTYPE", ctypes.CFUNCTYPE))
    return factory(ctypes.c_ssize_t, ctypes.c_int, ctypes.c_size_t, ctypes.c_ssize_t)
