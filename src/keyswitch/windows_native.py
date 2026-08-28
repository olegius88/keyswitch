"""Direct Win32 API binding used by :mod:`keyswitch.windows_backend`.

The module deliberately uses only the Python standard library. It is imported
only on Windows; the high-level backend is tested independently through its
small native API protocol.
"""

from __future__ import annotations

import ctypes
import ntpath
import sys
import threading
from collections.abc import Callable
from typing import Protocol, cast

from .backend import (
    ALT_MASK,
    CONTROL_MASK,
    LOCK_MASK,
    SHIFT_MASK,
    SUPER_MASK,
    ScreenAnchor,
)
from .windows_backend import (
    VK_CAPITAL,
    VK_CONTROL,
    VK_LCONTROL,
    VK_LMENU,
    VK_LSHIFT,
    VK_MENU,
    VK_RCONTROL,
    VK_RMENU,
    VK_RSHIFT,
    VK_SHIFT,
    NativeInput,
    NativeKeyEvent,
    WindowsBackendError,
)


WH_KEYBOARD_LL = 13
HC_ACTION = 0
WM_KEYDOWN = 0x0100
WM_KEYUP = 0x0101
WM_SYSKEYDOWN = 0x0104
WM_SYSKEYUP = 0x0105
WM_QUIT = 0x0012
WM_USER = 0x0400
WM_INPUTLANGCHANGEREQUEST = 0x0050
PM_NOREMOVE = 0x0000
LLKHF_EXTENDED = 0x01
LLKHF_INJECTED = 0x10
KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_SCANCODE = 0x0008
INPUT_KEYBOARD = 1
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
KEYSWITCH_EXTRA_INFO = 0x4B535743
GA_ROOT = 2
SW_RESTORE = 9


class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("vkCode", ctypes.c_ulong),
        ("scanCode", ctypes.c_ulong),
        ("flags", ctypes.c_ulong),
        ("time", ctypes.c_ulong),
        ("dwExtraInfo", ctypes.c_size_t),
    ]


class POINT(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]  # type: ignore[mutable-override]


class RECT(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("left", ctypes.c_long),
        ("top", ctypes.c_long),
        ("right", ctypes.c_long),
        ("bottom", ctypes.c_long),
    ]


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("cbSize", ctypes.c_ulong),
        ("flags", ctypes.c_ulong),
        ("hwndActive", ctypes.c_void_p),
        ("hwndFocus", ctypes.c_void_p),
        ("hwndCapture", ctypes.c_void_p),
        ("hwndMenuOwner", ctypes.c_void_p),
        ("hwndMoveSize", ctypes.c_void_p),
        ("hwndCaret", ctypes.c_void_p),
        ("rcCaret", RECT),
    ]


class MSG(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("hwnd", ctypes.c_void_p),
        ("message", ctypes.c_uint),
        ("wParam", ctypes.c_size_t),
        ("lParam", ctypes.c_ssize_t),
        ("time", ctypes.c_ulong),
        ("pt", POINT),
        ("lPrivate", ctypes.c_ulong),
    ]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("dx", ctypes.c_long),
        ("dy", ctypes.c_long),
        ("mouseData", ctypes.c_ulong),
        ("dwFlags", ctypes.c_ulong),
        ("time", ctypes.c_ulong),
        ("dwExtraInfo", ctypes.c_size_t),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("wVk", ctypes.c_ushort),
        ("wScan", ctypes.c_ushort),
        ("dwFlags", ctypes.c_ulong),
        ("time", ctypes.c_ulong),
        ("dwExtraInfo", ctypes.c_size_t),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("uMsg", ctypes.c_ulong),
        ("wParamL", ctypes.c_ushort),
        ("wParamH", ctypes.c_ushort),
    ]


class INPUTUNION(ctypes.Union):
    _fields_ = [  # type: ignore[mutable-override]
        ("mi", MOUSEINPUT),
        ("ki", KEYBDINPUT),
        ("hi", HARDWAREINPUT),
    ]


class INPUT(ctypes.Structure):
    _anonymous_ = ("union",)  # type: ignore[mutable-override]
    _fields_ = [("type", ctypes.c_ulong), ("union", INPUTUNION)]  # type: ignore[mutable-override]


class _HookCallback(Protocol):
    def __call__(self, code: int, message: int, data: int, /) -> int: ...


def _running_on_windows() -> bool:
    """Keep the runtime guard testable without platform-based type narrowing."""

    return sys.platform == "win32"


class CtypesWindowsAPI:
    """Small owner of User32/Kernel32 functions required by KeySwitch."""

    def __init__(self) -> None:
        if not _running_on_windows():
            raise WindowsBackendError("Win32 API загружается только в Windows")
        self.user32 = ctypes.CDLL("user32.dll", use_last_error=True)
        self.kernel32 = ctypes.CDLL("kernel32.dll", use_last_error=True)
        self._thread_id = 0
        self._hook: int | None = None
        self._hook_callback: _HookCallback | None = None
        self._lock = threading.Lock()
        self._declare()

    def _declare(self) -> None:
        user32 = self.user32
        kernel32 = self.kernel32
        self.hook_callback_type = ctypes.CFUNCTYPE(
            ctypes.c_ssize_t,
            ctypes.c_int,
            ctypes.c_size_t,
            ctypes.c_ssize_t,
        )
        user32.SetWindowsHookExW.argtypes = [
            ctypes.c_int,
            self.hook_callback_type,
            ctypes.c_void_p,
            ctypes.c_ulong,
        ]
        user32.SetWindowsHookExW.restype = ctypes.c_void_p
        user32.UnhookWindowsHookEx.argtypes = [ctypes.c_void_p]
        user32.UnhookWindowsHookEx.restype = ctypes.c_int
        user32.CallNextHookEx.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_size_t,
            ctypes.c_ssize_t,
        ]
        user32.CallNextHookEx.restype = ctypes.c_ssize_t
        user32.GetMessageW.argtypes = [
            ctypes.POINTER(MSG),
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_uint,
        ]
        user32.GetMessageW.restype = ctypes.c_int
        user32.PeekMessageW.argtypes = [
            ctypes.POINTER(MSG),
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_uint,
            ctypes.c_uint,
        ]
        user32.PeekMessageW.restype = ctypes.c_int
        user32.TranslateMessage.argtypes = [ctypes.POINTER(MSG)]
        user32.TranslateMessage.restype = ctypes.c_int
        user32.DispatchMessageW.argtypes = [ctypes.POINTER(MSG)]
        user32.DispatchMessageW.restype = ctypes.c_ssize_t
        user32.PostThreadMessageW.argtypes = [
            ctypes.c_ulong,
            ctypes.c_uint,
            ctypes.c_size_t,
            ctypes.c_ssize_t,
        ]
        user32.PostThreadMessageW.restype = ctypes.c_int
        user32.GetKeyboardLayoutList.argtypes = [
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_void_p),
        ]
        user32.GetKeyboardLayoutList.restype = ctypes.c_int
        user32.GetKeyboardLayout.argtypes = [ctypes.c_ulong]
        user32.GetKeyboardLayout.restype = ctypes.c_void_p
        user32.GetForegroundWindow.argtypes = []
        user32.GetForegroundWindow.restype = ctypes.c_void_p
        user32.GetAncestor.argtypes = [ctypes.c_void_p, ctypes.c_uint]
        user32.GetAncestor.restype = ctypes.c_void_p
        user32.SetForegroundWindow.argtypes = [ctypes.c_void_p]
        user32.SetForegroundWindow.restype = ctypes.c_int
        user32.ShowWindow.argtypes = [ctypes.c_void_p, ctypes.c_int]
        user32.ShowWindow.restype = ctypes.c_int
        user32.GetKeyState.argtypes = [ctypes.c_int]
        user32.GetKeyState.restype = ctypes.c_short
        user32.GetWindowThreadProcessId.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_ulong),
        ]
        user32.GetWindowThreadProcessId.restype = ctypes.c_ulong
        user32.GetGUIThreadInfo.argtypes = [
            ctypes.c_ulong,
            ctypes.POINTER(GUITHREADINFO),
        ]
        user32.GetGUIThreadInfo.restype = ctypes.c_int
        user32.ClientToScreen.argtypes = [ctypes.c_void_p, ctypes.POINTER(POINT)]
        user32.ClientToScreen.restype = ctypes.c_int
        user32.GetCursorPos.argtypes = [ctypes.POINTER(POINT)]
        user32.GetCursorPos.restype = ctypes.c_int
        user32.PostMessageW.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_size_t,
            ctypes.c_ssize_t,
        ]
        user32.PostMessageW.restype = ctypes.c_int
        user32.ToUnicodeEx.argtypes = [
            ctypes.c_uint,
            ctypes.c_uint,
            ctypes.POINTER(ctypes.c_ubyte),
            ctypes.c_wchar_p,
            ctypes.c_int,
            ctypes.c_uint,
            ctypes.c_void_p,
        ]
        user32.ToUnicodeEx.restype = ctypes.c_int
        user32.SendInput.argtypes = [
            ctypes.c_uint,
            ctypes.POINTER(INPUT),
            ctypes.c_int,
        ]
        user32.SendInput.restype = ctypes.c_uint
        kernel32.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
        kernel32.GetModuleHandleW.restype = ctypes.c_void_p
        kernel32.GetCurrentThreadId.argtypes = []
        kernel32.GetCurrentThreadId.restype = ctypes.c_ulong
        kernel32.GetLastError.argtypes = []
        kernel32.GetLastError.restype = ctypes.c_ulong
        kernel32.OpenProcess.argtypes = [
            ctypes.c_ulong,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        kernel32.OpenProcess.restype = ctypes.c_void_p
        kernel32.QueryFullProcessImageNameW.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_wchar_p,
            ctypes.POINTER(ctypes.c_ulong),
        ]
        kernel32.QueryFullProcessImageNameW.restype = ctypes.c_int
        kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
        kernel32.CloseHandle.restype = ctypes.c_int

    def _error(self, message: str) -> WindowsBackendError:
        code = int(self.kernel32.GetLastError())
        return WindowsBackendError(f"{message} (Win32 error {code})")

    def loaded_layouts(self) -> tuple[int, ...]:
        count = int(self.user32.GetKeyboardLayoutList(0, None))
        if count <= 0:
            raise self._error("Windows не вернула список раскладок")
        values = (ctypes.c_void_p * count)()
        actual = int(self.user32.GetKeyboardLayoutList(count, values))
        if actual <= 0:
            raise self._error("Windows не заполнила список раскладок")
        return tuple(int(values[index] or 0) for index in range(actual))

    def foreground_layout(self) -> int:
        window = self.user32.GetForegroundWindow()
        if not window:
            return 0
        thread_id = int(self.user32.GetWindowThreadProcessId(window, None))
        if not thread_id:
            return 0
        return int(self.user32.GetKeyboardLayout(thread_id) or 0)

    def activate_window(self, window: int) -> bool:
        """Restore and request foreground activation for a Tk child HWND."""

        handle = ctypes.c_void_p(window)
        root = self.user32.GetAncestor(handle, GA_ROOT) or handle
        self.user32.ShowWindow(root, SW_RESTORE)
        return bool(self.user32.SetForegroundWindow(root))

    def input_anchor(self) -> ScreenAnchor | None:
        window = int(self.user32.GetForegroundWindow() or 0)
        thread_id = (
            int(
                self.user32.GetWindowThreadProcessId(
                    ctypes.c_void_p(window), None
                )
            )
            if window
            else 0
        )
        info = GUITHREADINFO()
        info.cbSize = ctypes.sizeof(GUITHREADINFO)
        if thread_id and self.user32.GetGUIThreadInfo(
            thread_id, ctypes.byref(info)
        ):
            caret = int(info.hwndCaret or 0)
            point = POINT(info.rcCaret.left, info.rcCaret.bottom)
            if caret and self.user32.ClientToScreen(
                ctypes.c_void_p(caret), ctypes.byref(point)
            ):
                return ScreenAnchor(int(point.x), int(point.y), window or None)
        point = POINT()
        if self.user32.GetCursorPos(ctypes.byref(point)):
            return ScreenAnchor(int(point.x), int(point.y), window or None)
        return None

    def request_layout(self, layout: int) -> bool:
        window = self.user32.GetForegroundWindow()
        return bool(
            window
            and self.user32.PostMessageW(
                window,
                WM_INPUTLANGCHANGEREQUEST,
                0,
                layout,
            )
        )

    def translate_key(
        self,
        virtual_key: int,
        scan_code: int,
        state: int,
        layout: int,
    ) -> str:
        keys = (ctypes.c_ubyte * 256)()
        if state & SHIFT_MASK:
            for key in (VK_SHIFT, VK_LSHIFT, VK_RSHIFT):
                keys[key] = 0x80
        if state & CONTROL_MASK:
            for key in (VK_CONTROL, VK_LCONTROL, VK_RCONTROL):
                keys[key] = 0x80
        if state & ALT_MASK:
            for key in (VK_MENU, VK_LMENU, VK_RMENU):
                keys[key] = 0x80
        if state & LOCK_MASK:
            keys[VK_CAPITAL] = 0x01
        if state & SUPER_MASK:
            return ""
        keys[virtual_key & 0xFF] |= 0x80
        buffer = ctypes.create_unicode_buffer(8)
        written = int(
            self.user32.ToUnicodeEx(
                virtual_key,
                scan_code,
                keys,
                buffer,
                len(buffer),
                0x04,
                layout,
            )
        )
        if written <= 0:
            return ""
        result = buffer.value[:written]
        return result if result.isprintable() else ""

    def send_inputs(self, inputs: tuple[NativeInput, ...]) -> int:
        if not inputs:
            return 0
        native = (INPUT * len(inputs))()
        for index, item in enumerate(inputs):
            flags = 0
            if not item.pressed:
                flags |= KEYEVENTF_KEYUP
            if item.extended:
                flags |= KEYEVENTF_EXTENDEDKEY
            if item.scan_code:
                flags |= KEYEVENTF_SCANCODE
            native[index].type = INPUT_KEYBOARD
            native[index].ki = KEYBDINPUT(
                item.virtual_key,
                item.scan_code,
                flags,
                0,
                KEYSWITCH_EXTRA_INFO if item.synthetic else 0,
            )
        return int(self.user32.SendInput(len(native), native, ctypes.sizeof(INPUT)))

    def active_application(self) -> str:
        window = self.user32.GetForegroundWindow()
        if not window:
            return ""
        process_id = ctypes.c_ulong()
        if not self.user32.GetWindowThreadProcessId(window, ctypes.byref(process_id)):
            return ""
        process = self.kernel32.OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            process_id.value,
        )
        if not process:
            return ""
        try:
            size = ctypes.c_ulong(32768)
            buffer = ctypes.create_unicode_buffer(size.value)
            if not self.kernel32.QueryFullProcessImageNameW(
                process,
                0,
                buffer,
                ctypes.byref(size),
            ):
                return ""
            return ntpath.splitext(ntpath.basename(buffer.value))[0]
        finally:
            self.kernel32.CloseHandle(process)

    def caps_lock_enabled(self) -> bool:
        return bool(int(self.user32.GetKeyState(VK_CAPITAL)) & 1)

    def run_keyboard_hook(
        self,
        listener: Callable[[NativeKeyEvent], None],
        ready: Callable[[], None],
    ) -> None:
        def callback(code: int, message: int, data: int) -> int:
            if code == HC_ACTION and message in {
                WM_KEYDOWN,
                WM_KEYUP,
                WM_SYSKEYDOWN,
                WM_SYSKEYUP,
            }:
                native = ctypes.cast(
                    data,
                    ctypes.POINTER(KBDLLHOOKSTRUCT),
                ).contents
                listener(
                    NativeKeyEvent(
                        message in {WM_KEYDOWN, WM_SYSKEYDOWN},
                        int(native.vkCode),
                        int(native.scanCode),
                        bool(native.flags & LLKHF_EXTENDED),
                        bool(
                            native.flags & LLKHF_INJECTED
                            and native.dwExtraInfo == KEYSWITCH_EXTRA_INFO
                        ),
                        int(native.time),
                    )
                )
            return int(self.user32.CallNextHookEx(None, code, message, data))

        callback_object = self.hook_callback_type(callback)
        self._hook_callback = cast(_HookCallback, callback_object)
        # PostThreadMessageW fails when the target thread has no message queue.
        # Create it before publishing readiness so even an immediate stop is
        # guaranteed to deliver WM_QUIT.
        queue_message = MSG()
        self.user32.PeekMessageW(
            ctypes.byref(queue_message),
            None,
            WM_USER,
            WM_USER,
            PM_NOREMOVE,
        )
        module = self.kernel32.GetModuleHandleW(None)
        hook = self.user32.SetWindowsHookExW(
            WH_KEYBOARD_LL,
            callback_object,
            module,
            0,
        )
        if not hook:
            self._hook_callback = None
            raise self._error("Не удалось установить WH_KEYBOARD_LL")
        with self._lock:
            self._hook = int(hook)
            self._thread_id = int(self.kernel32.GetCurrentThreadId())
        ready()
        message = MSG()
        try:
            while True:
                result = int(self.user32.GetMessageW(ctypes.byref(message), None, 0, 0))
                if result == 0:
                    break
                if result < 0:
                    raise self._error("GetMessageW завершился ошибкой")
                self.user32.TranslateMessage(ctypes.byref(message))
                self.user32.DispatchMessageW(ctypes.byref(message))
        finally:
            self.user32.UnhookWindowsHookEx(hook)
            with self._lock:
                self._hook = None
                self._thread_id = 0
            self._hook_callback = None

    def stop_keyboard_hook(self) -> None:
        with self._lock:
            thread_id = self._thread_id
        if thread_id:
            self.user32.PostThreadMessageW(thread_id, WM_QUIT, 0, 0)
