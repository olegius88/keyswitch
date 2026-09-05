"""Read a bounded caret range through Windows UI Automation.

Called on the engine worker, never WH_KEYBOARD_LL. IUIAutomation2 timeouts
bound provider calls. Password and selected fields are identified before any
GetText call. comtypes interfaces are generated from the OS type library.
See Microsoft UI Automation TextPattern Overview / IUIAutomation2.
"""

from __future__ import annotations

import ctypes
import importlib
import sys
import threading
from collections.abc import Sequence
from typing import Protocol, cast

from .input_context import CONTEXT_LIMIT, FieldContext, FieldRole


class _Range(Protocol):
    def Clone(self) -> _Range: ...
    def CompareEndpoints(self, endpoint: int, other: _Range, other_endpoint: int) -> int: ...
    def MoveEndpointByUnit(self, endpoint: int, unit: int, count: int) -> int: ...
    def GetText(self, maximum: int) -> str: ...


class _Ranges(Protocol):
    @property
    def Length(self) -> int: ...
    def GetElement(self, index: int) -> _Range: ...


class _TextPattern(Protocol):
    def GetSelection(self) -> _Ranges: ...


class _Unknown(Protocol):
    def QueryInterface(self, interface: object) -> _TextPattern: ...


class _Element(Protocol):
    @property
    def CurrentProcessId(self) -> int: ...
    @property
    def CurrentIsPassword(self) -> bool: ...
    @property
    def CurrentAutomationId(self) -> str: ...
    def GetRuntimeId(self) -> Sequence[int]: ...
    def GetCurrentPattern(self, pattern: int) -> _Unknown: ...


class _Automation(Protocol):
    ConnectionTimeout: int
    TransactionTimeout: int
    def GetFocusedElement(self) -> _Element: ...


class _TypeLibrary(Protocol):
    @property
    def CUIAutomation8(self) -> object: ...
    @property
    def IUIAutomation2(self) -> object: ...
    @property
    def IUIAutomationTextPattern(self) -> object: ...


class _ComClient(Protocol):
    def GetModule(self, name: str) -> _TypeLibrary: ...
    def CreateObject(self, cls: object, *, interface: object) -> _Automation: ...


class _Com(Protocol):
    def CoInitializeEx(self, flags: int) -> None: ...
    def CoUninitialize(self) -> None: ...


class WindowsFieldReader:
    def __init__(self, automation: _Automation | None = None, text_interface: object = None) -> None:
        self._com: _Com | None = None
        self._thread = threading.get_ident()
        if automation is None:
            # The worker thread has its own COM apartment. No UIA objects are
            # passed between the hook/UI and worker threads.
            # comtypes initializes the *first importing thread* automatically.
            # Select MTA for that import, or explicitly initialize a later
            # worker. Otherwise its default STA conflicts with UIA's MTA.
            imported = "comtypes" in sys.modules
            previous = sys.__dict__.get("coinit_flags")
            sys.__dict__["coinit_flags"] = 0
            try:
                com = cast(_Com, importlib.import_module("comtypes"))
            finally:
                if previous is None:
                    sys.__dict__.pop("coinit_flags", None)
                else:
                    sys.__dict__["coinit_flags"] = previous
            if imported:
                com.CoInitializeEx(0)
            self._com = com
            try:
                client = cast(_ComClient, importlib.import_module("comtypes.client"))
                library = client.GetModule("UIAutomationCore.dll")
                automation = client.CreateObject(library.CUIAutomation8, interface=library.IUIAutomation2)
                text_interface = library.IUIAutomationTextPattern
            except Exception:
                self.close()
                raise
        self.automation = automation
        self.text_interface = text_interface
        try:
            self.automation.ConnectionTimeout = 50
            self.automation.TransactionTimeout = 50
        except Exception:
            self.close()
            raise

    def close(self) -> None:
        if self._com is not None and threading.get_ident() == self._thread:
            # Drop all apartment-bound interface references before uninit.
            self.__dict__.pop("automation", None)
            self._com.CoUninitialize()
            self._com = None

    @staticmethod
    def _process_for_window(window: int) -> int:
        # KeySwitch ships x64 only; like windows_native, use the x64 unified
        # calling convention. CDLL is also available to cross-platform mocks.
        user32 = ctypes.CDLL("user32.dll", use_last_error=True)
        user32.GetWindowThreadProcessId.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong)]
        user32.GetWindowThreadProcessId.restype = ctypes.c_ulong
        pid = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(window, ctypes.byref(pid))
        return pid.value

    def read(self, application: str, window: int) -> FieldContext | None:
        element = self.automation.GetFocusedElement()
        if element.CurrentProcessId != self._process_for_window(window):
            return None
        identity = tuple(element.GetRuntimeId())
        field_id = ":".join(str(value) for value in identity)
        if element.CurrentIsPassword:
            return FieldContext(application, field_id, role="password", sensitive=True, source="uia")
        pattern = element.GetCurrentPattern(10014).QueryInterface(self.text_interface)
        ranges = pattern.GetSelection()
        if ranges.Length > 1:
            return FieldContext(application, field_id, selection=True, source="uia")
        if ranges.Length != 1:
            return None
        caret = ranges.GetElement(0)
        if caret.CompareEndpoints(0, caret, 1) != 0:
            return FieldContext(application, field_id, selection=True, source="uia")
        before, after = caret.Clone(), caret.Clone()
        before.MoveEndpointByUnit(0, 0, -CONTEXT_LIMIT)
        after.MoveEndpointByUnit(1, 0, 128)
        prefix, suffix = before.GetText(CONTEXT_LIMIT), after.GetText(128)
        current = self.automation.GetFocusedElement()
        if current.CurrentIsPassword:
            return FieldContext(application, field_id, role="password", sensitive=True, source="uia")
        if tuple(current.GetRuntimeId()) != identity:
            # Explicit contradictory evidence, not an unsupported provider.
            # An empty native suffix makes the policy reject stale strokes.
            return FieldContext(application, field_id, source="uia")
        role: FieldRole = "search" if "search" in element.CurrentAutomationId.casefold() else "text"
        return FieldContext(application, field_id, prefix, suffix, role, source="uia").bounded()


def probe_uia() -> dict[str, object]:
    """Explicit diagnostic readiness probe; no focused field or text is read."""
    try:
        reader = WindowsFieldReader()
        reader.close()
    except Exception as error:
        return {"available": False, "error": type(error).__name__}
    return {"available": True}
