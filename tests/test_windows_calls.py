"""The calling convention of KeySwitch's ctypes bindings to the Win32 API (keyswitch.windows_calls).

On 32-bit x86 Windows user32 and kernel32 are `__stdcall`, as is a hook procedure, and ctypes follows that
convention only through `WinDLL` and `WINFUNCTYPE`; a cdecl binding there corrupts the stack on the first call.
"""

from __future__ import annotations

import ctypes
import sys
import unittest
from unittest.mock import MagicMock, patch

from keyswitch.windows_calls import hook_prototype, stdcall_library


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


if __name__ == "__main__":
    unittest.main()
