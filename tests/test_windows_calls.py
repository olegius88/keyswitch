"""The calling convention of KeySwitch's ctypes bindings to the Win32 API (keyswitch.windows_calls).

On 32-bit x86 Windows user32 and kernel32 are `__stdcall`, as is a hook procedure, and ctypes follows that
convention only through `WinDLL` and `WINFUNCTYPE`; a cdecl binding there corrupts the stack on the first call.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import os
import sys
import unittest
from unittest.mock import MagicMock, patch

from _ctypes import FUNCFLAG_PYTHONAPI

from keyswitch.windows_calls import (
    GIL_HOLDING_CALLS,
    gil_holding_library,
    hook_prototype,
    keep_gil_in_short_calls,
    stdcall_library,
)


class StdcallBindingTests(unittest.TestCase):
    def test_windows_binds_system_libraries_and_hooks_with_the_stdcall_convention(self) -> None:
        loader, factory = MagicMock(), MagicMock()
        with patch.object(ctypes, "WinDLL", loader, create=True), patch.object(ctypes, "WINFUNCTYPE", factory, create=True):
            self.assertIs(stdcall_library(), loader)
            self.assertIs(hook_prototype(), factory.return_value)
        factory.assert_called_once_with(ctypes.c_ssize_t, ctypes.c_int, ctypes.c_size_t, ctypes.c_ssize_t)

    def test_without_the_windows_loaders_the_bindings_are_cdecl(self) -> None:
        with patch.dict(vars(ctypes)):
            vars(ctypes).pop("WinDLL", None)
            vars(ctypes).pop("WINFUNCTYPE", None)
            self.assertIs(stdcall_library(), ctypes.CDLL)
            prototype = hook_prototype()
        self.assertIs(getattr(prototype, "_restype_"), ctypes.c_ssize_t)
        self.assertEqual(getattr(prototype, "_argtypes_"), (ctypes.c_int, ctypes.c_size_t, ctypes.c_ssize_t))

    @unittest.skipUnless(sys.platform == "win32", "ctypes has WinDLL only on Windows")
    def test_on_windows_the_bindings_are_the_stdcall_ones(self) -> None:
        self.assertIs(stdcall_library(), getattr(ctypes, "WinDLL"))
        self.assertEqual(getattr(hook_prototype(), "_flags_"), getattr(getattr(ctypes, "WINFUNCTYPE")(ctypes.c_int), "_flags_"))


class GilHoldingBindingTests(unittest.TestCase):
    """Short calls on the keyboard hook's path keep the GIL, in the calling convention of the system libraries."""

    def test_windows_adds_the_python_api_flag_to_the_stdcall_loader(self) -> None:
        class StdcallLoader(ctypes.CDLL):
            _func_flags_ = 0

        with patch.object(ctypes, "WinDLL", StdcallLoader, create=True):
            library = gil_holding_library()
        self.assertTrue(issubclass(library, StdcallLoader))
        self.assertEqual(library._func_flags_, FUNCFLAG_PYTHONAPI)

    def test_without_the_windows_loader_it_is_the_cdecl_loader_that_keeps_the_gil(self) -> None:
        with patch.dict(vars(ctypes)):
            vars(ctypes).pop("WinDLL", None)
            library = gil_holding_library()
        self.assertTrue(issubclass(library, ctypes.CDLL))
        self.assertEqual(library._func_flags_, ctypes.PyDLL._func_flags_)

    def test_the_short_calls_are_rebound_with_their_declared_types(self) -> None:
        if sys.platform == "win32":
            name, function_name = "kernel32.dll", "GetCurrentProcessId"
        else:
            name, function_name = str(ctypes.util.find_library("c")), "getpid"
        library = stdcall_library()(name, use_last_error=True)
        released = getattr(library, function_name)
        released.argtypes = []
        released.restype = ctypes.c_ulong
        with patch.dict(GIL_HOLDING_CALLS, {name: (function_name,)}, clear=True):
            keep_gil_in_short_calls(library, name)
        kept = getattr(library, function_name)
        self.assertIsNot(kept, released)
        self.assertTrue(kept._flags_ & FUNCFLAG_PYTHONAPI)
        self.assertFalse(released._flags_ & FUNCFLAG_PYTHONAPI)
        self.assertEqual(kept.argtypes, [])
        self.assertIs(kept.restype, ctypes.c_ulong)
        self.assertEqual(kept(), os.getpid())

    def test_no_call_that_waits_or_reaches_the_hook_keeps_the_gil(self) -> None:
        # Holding the GIL across one of these would stop the hook thread the call is waiting for.
        waiting = {
            "AttachThreadInput", "BringWindowToTop", "CallNextHookEx", "DispatchMessageW", "GetMessageW",
            "PeekMessageW", "PostMessageW", "PostThreadMessageW", "SendInput", "SendMessageW",
            "SetForegroundWindow", "SetWindowPos", "SetWindowsHookExW", "UnhookWindowsHookEx",
        }
        named = {function for functions in GIL_HOLDING_CALLS.values() for function in functions}
        self.assertFalse(named & waiting)
        self.assertEqual(set(GIL_HOLDING_CALLS), {"user32.dll", "kernel32.dll"})

    @unittest.skipUnless(sys.platform == "win32", "ctypes has WinDLL only on Windows")
    def test_on_windows_a_kernel32_call_keeps_the_stdcall_convention(self) -> None:
        windll = getattr(ctypes, "WinDLL")
        library = gil_holding_library()
        self.assertEqual(library._func_flags_, windll._func_flags_ | FUNCFLAG_PYTHONAPI)
        kernel32 = library("kernel32.dll", use_last_error=True)
        kernel32.GetCurrentProcessId.restype = ctypes.c_ulong
        self.assertEqual(kernel32.GetCurrentProcessId(), windll("kernel32.dll").GetCurrentProcessId())


if __name__ == "__main__":
    unittest.main()
