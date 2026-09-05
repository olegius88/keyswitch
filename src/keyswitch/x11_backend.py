"""X11 RECORD/XKB/XTEST backend implemented against the system libraries.

The module intentionally has no python-xlib dependency. ctypes declarations
mirror the X11 headers shipped by Ubuntu and are covered by a live probe in the
diagnostics command.
"""

from __future__ import annotations

import ctypes
from collections.abc import Sequence
import ctypes.util
import logging
import os
import struct
import threading
import time
from collections import deque
from typing import Callable, Iterable, Protocol, cast

from .backend import (
    ALT_MASK,
    CONTROL_MASK as CONTROL_MASK,
    LOCK_MASK as LOCK_MASK,
    SHIFT_MASK as SHIFT_MASK,
    SUPER_MASK,
    BackendProbe as BackendProbe,
    FocusInfo,
    KeyEvent as KeyEvent,
    KeyDisposition,
    ScreenAnchor,
)


LOGGER = logging.getLogger(__name__)

KEY_PRESS = 2
BUTTON_PRESS = 4
KEY_RELEASE = 3
XRECORD_FROM_SERVER = 0
XRECORD_START_OF_DATA = 4
XRECORD_ALL_CLIENTS = 3
XKB_USE_CORE_KBD = 0x0100
XRECORD_START_TIMEOUT = 5.0
REVERT_TO_PARENT = 2
CURRENT_TIME = 0
XA_CARDINAL = 6

MOD1_MASK = ALT_MASK
MOD4_MASK = SUPER_MASK


class XRecordRange8(ctypes.Structure):
    _fields_ = [("first", ctypes.c_ubyte), ("last", ctypes.c_ubyte)]  # type: ignore[mutable-override]


class XRecordRange16(ctypes.Structure):
    _fields_ = [("first", ctypes.c_ushort), ("last", ctypes.c_ushort)]  # type: ignore[mutable-override]


class XRecordExtRange(ctypes.Structure):
    _fields_ = [("ext_major", XRecordRange8), ("ext_minor", XRecordRange16)]  # type: ignore[mutable-override]


class XRecordRange(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("core_requests", XRecordRange8),
        ("core_replies", XRecordRange8),
        ("ext_requests", XRecordExtRange),
        ("ext_replies", XRecordExtRange),
        ("delivered_events", XRecordRange8),
        ("device_events", XRecordRange8),
        ("errors", XRecordRange8),
        ("client_started", ctypes.c_int),
        ("client_died", ctypes.c_int),
    ]


class XRecordInterceptData(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("id_base", ctypes.c_ulong),
        ("server_time", ctypes.c_ulong),
        ("client_seq", ctypes.c_ulong),
        ("category", ctypes.c_int),
        ("client_swapped", ctypes.c_int),
        ("data", ctypes.POINTER(ctypes.c_ubyte)),
        ("data_len", ctypes.c_ulong),
    ]


class XkbStateRec(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("group", ctypes.c_ubyte),
        ("locked_group", ctypes.c_ubyte),
        ("base_group", ctypes.c_ushort),
        ("latched_group", ctypes.c_ushort),
        ("mods", ctypes.c_ubyte),
        ("base_mods", ctypes.c_ubyte),
        ("latched_mods", ctypes.c_ubyte),
        ("locked_mods", ctypes.c_ubyte),
        ("compat_state", ctypes.c_ubyte),
        ("grab_mods", ctypes.c_ubyte),
        ("compat_grab_mods", ctypes.c_ubyte),
        ("lookup_mods", ctypes.c_ubyte),
        ("compat_lookup_mods", ctypes.c_ubyte),
        ("ptr_buttons", ctypes.c_ushort),
    ]


class XClassHint(ctypes.Structure):
    _fields_ = [("res_name", ctypes.c_void_p), ("res_class", ctypes.c_void_p)]  # type: ignore[mutable-override]


class _RecordCallback(Protocol):
    def __call__(
        self,
        closure: int | None,
        data_pointer: ctypes._Pointer[XRecordInterceptData],
        /,
    ) -> None: ...


class X11Error(RuntimeError):
    pass


class _Libraries:
    _initialized = False
    _lock = threading.Lock()

    def __init__(self) -> None:
        x11_name = ctypes.util.find_library("X11")
        xtst_name = ctypes.util.find_library("Xtst")
        xkb_name = ctypes.util.find_library("xkbcommon")
        if not x11_name or not xtst_name or not xkb_name:
            raise X11Error("Не найдены системные библиотеки X11, Xtst или xkbcommon")
        self.x11 = ctypes.CDLL(x11_name)
        self.xtst = ctypes.CDLL(xtst_name)
        self.xkb = ctypes.CDLL(xkb_name)
        self._declare()
        with self._lock:
            if not self.__class__._initialized:
                if not self.x11.XInitThreads():
                    raise X11Error("XInitThreads завершился ошибкой")
                self.__class__._initialized = True

    def _declare(self) -> None:
        x11, xtst, xkb = self.x11, self.xtst, self.xkb
        x11.XInitThreads.argtypes = []
        x11.XInitThreads.restype = ctypes.c_int
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
        x11.XCloseDisplay.restype = ctypes.c_int
        x11.XFlush.argtypes = [ctypes.c_void_p]
        x11.XFlush.restype = ctypes.c_int
        x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        x11.XSync.restype = ctypes.c_int
        x11.XFree.argtypes = [ctypes.c_void_p]
        x11.XFree.restype = ctypes.c_int
        x11.XkbKeycodeToKeysym.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ubyte,
            ctypes.c_int,
            ctypes.c_int,
        ]
        x11.XkbKeycodeToKeysym.restype = ctypes.c_ulong
        x11.XkbLookupKeySym.argtypes = [
            ctypes.c_void_p, ctypes.c_ubyte, ctypes.c_uint,
            ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_ulong),
        ]
        x11.XkbLookupKeySym.restype = ctypes.c_int
        x11.XkbQueryExtension.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
        ]
        x11.XkbQueryExtension.restype = ctypes.c_int
        x11.XkbLockGroup.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint]
        x11.XkbLockGroup.restype = ctypes.c_int
        x11.XkbGetState.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.POINTER(XkbStateRec)]
        x11.XkbGetState.restype = ctypes.c_int
        x11.XKeysymToString.argtypes = [ctypes.c_ulong]
        x11.XKeysymToString.restype = ctypes.c_char_p
        x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        x11.XKeysymToKeycode.restype = ctypes.c_ubyte
        x11.XGetInputFocus.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_int),
        ]
        x11.XGetInputFocus.restype = ctypes.c_int
        x11.XSetInputFocus.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        x11.XSetInputFocus.restype = ctypes.c_int
        x11.XDefaultScreen.argtypes = [ctypes.c_void_p]
        x11.XDefaultScreen.restype = ctypes.c_int
        x11.XRootWindow.argtypes = [ctypes.c_void_p, ctypes.c_int]
        x11.XRootWindow.restype = ctypes.c_ulong
        x11.XQueryPointer.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_uint),
        ]
        x11.XQueryPointer.restype = ctypes.c_int
        x11.XMoveWindow.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_int,
            ctypes.c_int,
        ]
        x11.XMoveWindow.restype = ctypes.c_int
        x11.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        x11.XRaiseWindow.restype = ctypes.c_int
        x11.XGetClassHint.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(XClassHint)]
        x11.XGetClassHint.restype = ctypes.c_int
        x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        x11.XInternAtom.restype = ctypes.c_ulong
        x11.XGetWindowProperty.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_ulong,
            ctypes.c_long,
            ctypes.c_long,
            ctypes.c_int,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_void_p),
        ]
        x11.XGetWindowProperty.restype = ctypes.c_int
        x11.XQueryTree.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
            ctypes.POINTER(ctypes.c_uint),
        ]
        x11.XQueryTree.restype = ctypes.c_int

        xtst.XRecordQueryVersion.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
        ]
        xtst.XRecordQueryVersion.restype = ctypes.c_int
        xtst.XRecordAllocRange.argtypes = []
        xtst.XRecordAllocRange.restype = ctypes.POINTER(XRecordRange)
        xtst.XRecordCreateContext.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.c_int,
            ctypes.POINTER(ctypes.POINTER(XRecordRange)),
            ctypes.c_int,
        ]
        xtst.XRecordCreateContext.restype = ctypes.c_ulong
        self.record_callback_type = ctypes.CFUNCTYPE(
            None, ctypes.c_void_p, ctypes.POINTER(XRecordInterceptData)
        )
        xtst.XRecordEnableContext.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            self.record_callback_type,
            ctypes.c_void_p,
        ]
        xtst.XRecordEnableContext.restype = ctypes.c_int
        xtst.XRecordDisableContext.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        xtst.XRecordDisableContext.restype = ctypes.c_int
        xtst.XRecordFreeContext.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        xtst.XRecordFreeContext.restype = ctypes.c_int
        xtst.XRecordFreeData.argtypes = [ctypes.POINTER(XRecordInterceptData)]
        xtst.XRecordFreeData.restype = None
        xtst.XTestQueryExtension.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
        ]
        xtst.XTestQueryExtension.restype = ctypes.c_int
        xtst.XTestFakeKeyEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        xtst.XTestFakeKeyEvent.restype = ctypes.c_int
        xtst.XTestGrabControl.argtypes = [ctypes.c_void_p, ctypes.c_int]
        xtst.XTestGrabControl.restype = ctypes.c_int
        xkb.xkb_keysym_to_utf32.argtypes = [ctypes.c_uint32]
        xkb.xkb_keysym_to_utf32.restype = ctypes.c_uint32


class X11Backend:
    """Observe all core keyboard events and inject deterministic corrections."""

    def __init__(self, group_count: int = 2) -> None:
        self.group_count = max(2, min(group_count, 4))
        self._libraries = _Libraries()
        self._control: int | None = None
        # Own-window verdicts by X window id; ids of another client can never
        # collide with ours (each client allocates from its own id range).
        self._own_windows: dict[int, bool] = {}
        self._net_wm_pid_atom: int | None = None
        self._record: int | None = None
        self._context = 0
        self._range: ctypes._Pointer[XRecordRange] | None = None
        self._record_callback: _RecordCallback | None = None
        self._thread: threading.Thread | None = None
        self._listener: Callable[[KeyEvent], None] | None = None
        self._running = threading.Event()
        self._capture_ready = threading.Event()
        self._capture_start_finished = threading.Event()
        self._expected: deque[tuple[bool, int]] = deque()
        self._expected_deadline = 0.0
        self._expected_lock = threading.Lock()
        self._inject_lock = threading.Lock()
        self._xkb_version = "—"

    @property
    def running(self) -> bool:
        return self._running.is_set()

    def _open(self) -> None:
        if os.environ.get("XDG_SESSION_TYPE", "x11").casefold() == "wayland":
            raise X11Error("Активна Wayland-сессия; backend этой версии работает через X11")
        display_name = os.environ.get("DISPLAY")
        if not display_name:
            raise X11Error("Переменная DISPLAY не задана")
        encoded = display_name.encode()
        self._control = self._libraries.x11.XOpenDisplay(encoded)
        self._record = self._libraries.x11.XOpenDisplay(encoded)
        if not self._control or not self._record:
            self.close()
            raise X11Error(f"Не удалось открыть X11 display {display_name}")
        opcode, event, error = ctypes.c_int(), ctypes.c_int(), ctypes.c_int()
        major, minor = ctypes.c_int(1), ctypes.c_int(0)
        if not self._libraries.x11.XkbQueryExtension(
            self._control,
            ctypes.byref(opcode),
            ctypes.byref(event),
            ctypes.byref(error),
            ctypes.byref(major),
            ctypes.byref(minor),
        ):
            self.close()
            raise X11Error("Расширение XKB недоступно или несовместимо")
        self._xkb_version = f"{major.value}.{minor.value}"

    def probe(self) -> BackendProbe:
        temporary = not self._control
        try:
            if temporary:
                self._open()
            assert self._control
            major, minor = ctypes.c_int(), ctypes.c_int()
            if not self._libraries.xtst.XRecordQueryVersion(
                self._control, ctypes.byref(major), ctypes.byref(minor)
            ):
                raise X11Error("Расширение XRecord недоступно")
            event_base, error_base = ctypes.c_int(), ctypes.c_int()
            xtest_major, xtest_minor = ctypes.c_int(), ctypes.c_int()
            if not self._libraries.xtst.XTestQueryExtension(
                self._control,
                ctypes.byref(event_base),
                ctypes.byref(error_base),
                ctypes.byref(xtest_major),
                ctypes.byref(xtest_minor),
            ):
                raise X11Error("Расширение XTEST недоступно")
            return BackendProbe(
                True,
                os.environ.get("XDG_SESSION_TYPE", "x11"),
                os.environ.get("DISPLAY", ""),
                f"{major.value}.{minor.value}",
                f"{xtest_major.value}.{xtest_minor.value}",
                self._xkb_version,
                self.current_group(),
            )
        except Exception as error:
            return BackendProbe(
                False,
                os.environ.get("XDG_SESSION_TYPE", "неизвестно"),
                os.environ.get("DISPLAY", ""),
                "—",
                "—",
                "—",
                -1,
                str(error),
            )
        finally:
            if temporary:
                self.close()

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], KeyDisposition] | None
    ) -> None:
        """Accepted for the shared backend contract and deliberately ignored.

        XRecord is a passive observer: X11 delivers the key to the focused
        client no matter what this process decides, so a key can never be
        withheld the way the Windows hook withholds it.
        """

        del predicate

    def complete_action(self, deliver: bool) -> int:
        """XRecord cannot defer actions; there is nothing to deliver here."""
        return 0

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        if self.running:
            return
        self._listener = listener
        if not self._control or not self._record:
            self._open()
        assert self._control and self._record
        major, minor = ctypes.c_int(), ctypes.c_int()
        if not self._libraries.xtst.XRecordQueryVersion(
            self._control, ctypes.byref(major), ctypes.byref(minor)
        ):
            self.close()
            raise X11Error("XRecord не поддерживается X-сервером")
        self._range = self._libraries.xtst.XRecordAllocRange()
        if not self._range:
            self.close()
            raise X11Error("XRecordAllocRange не выделил диапазон событий")
        self._range.contents.device_events.first = KEY_PRESS
        self._range.contents.device_events.last = BUTTON_PRESS
        clients = (ctypes.c_ulong * 1)(XRECORD_ALL_CLIENTS)
        ranges = (ctypes.POINTER(XRecordRange) * 1)(self._range)
        self._context = self._libraries.xtst.XRecordCreateContext(
            self._control, 0, clients, 1, ranges, 1
        )
        if not self._context:
            self.close()
            raise X11Error("Не удалось создать контекст XRecord")
        # XRecord uses a separate data connection. Flush the context creation
        # on the control connection before the data connection enables it.
        self._libraries.x11.XSync(self._control, 0)
        self._record_callback = cast(
            _RecordCallback,
            self._libraries.record_callback_type(self._handle_record_data),
        )
        self._capture_ready.clear()
        self._capture_start_finished.clear()
        self._running.set()
        self._thread = threading.Thread(
            target=self._record_loop,
            name="keyswitch-xrecord",
            daemon=True,
        )
        self._thread.start()
        if not self._capture_start_finished.wait(XRECORD_START_TIMEOUT):
            self.close()
            raise X11Error("XRecord не подтвердил запуск за 5 секунд")
        if not self._capture_ready.is_set():
            self.close()
            raise X11Error("XRecord завершился до подтверждения запуска")

    def _record_loop(self) -> None:
        try:
            assert self._record and self._record_callback
            status = self._libraries.xtst.XRecordEnableContext(
                self._record, self._context, self._record_callback, None
            )
            if self._running.is_set() and not status:
                LOGGER.error("XRecordEnableContext завершился ошибкой")
        except Exception:
            LOGGER.exception("Ошибка цикла XRecord")
        finally:
            self._running.clear()
            self._capture_start_finished.set()

    def _handle_record_data(
        self,
        _closure: int | None,
        data_pointer: ctypes._Pointer[XRecordInterceptData],
    ) -> None:
        try:
            data = data_pointer.contents
            if data.category == XRECORD_START_OF_DATA:
                self._capture_ready.set()
                self._capture_start_finished.set()
            if (
                data.category != XRECORD_FROM_SERVER
                or data.client_swapped
                or not data.data
                or not data.data_len
            ):
                return
            payload = ctypes.string_at(data.data, data.data_len * 4)
            for offset in range(0, len(payload) - 31, 32):
                event = self._decode_event(payload[offset : offset + 32])
                if event is not None and self._listener is not None:
                    self._listener(event)
        except Exception:
            LOGGER.exception("Не удалось разобрать событие XRecord")
        finally:
            self._libraries.xtst.XRecordFreeData(data_pointer)

    def _decode_event(self, payload: bytes) -> KeyEvent | None:
        (
            event_type,
            keycode,
            _sequence,
            timestamp,
            _root,
            _event,
            _child,
            _root_x,
            _root_y,
            _event_x,
            _event_y,
            state,
            _same_screen,
            _pad,
        ) = struct.unpack("=BBHIIIIhhhhHBB", payload)
        event_type &= 0x7F
        if event_type == BUTTON_PRESS:
            return KeyEvent(True, 0, "Pointer", "", ("", ""), -1, 0, timestamp)
        if event_type not in (KEY_PRESS, KEY_RELEASE):
            return None
        pressed = event_type == KEY_PRESS
        group = (state >> 13) & 0x3
        characters = tuple(
            self._character_for_keycode(keycode, candidate_group, state)
            for candidate_group in range(self.group_count)
        )
        character = characters[group] if group < len(characters) else ""
        key_name = self._key_name(keycode)
        if key_name == "Return":
            character = "\n"
        elif key_name in {"Tab", "ISO_Left_Tab"}:
            character = "\t"
        synthetic = self._consume_expected(pressed, keycode)
        return KeyEvent(
            pressed,
            keycode,
            key_name,
            character,
            characters,
            group,
            state,
            timestamp,
            synthetic,
        )

    def _character_for_keycode(self, keycode: int, group: int, state: int) -> str:
        if not self._control:
            return ""
        shift = bool(state & SHIFT_MASK)
        caps = bool(state & LOCK_MASK)
        keysym = self._libraries.x11.XkbKeycodeToKeysym(
            self._control, keycode, group, 1 if shift else 0
        )
        if not keysym:
            # Common keys such as Space often have only group 0 in XKB.
            # Direct indexing returns NoSymbol for RU; the actual key lookup
            # applies the key's group fallback, just as the editor does.
            consumed, resolved = ctypes.c_uint(), ctypes.c_ulong()
            lookup_state = (state & ~(3 << 13)) | ((group & 3) << 13)
            if not self._libraries.x11.XkbLookupKeySym(
                self._control, keycode, lookup_state,
                ctypes.byref(consumed), ctypes.byref(resolved),
            ):
                return ""
            keysym = resolved.value
        codepoint = self._libraries.xkb.xkb_keysym_to_utf32(keysym)
        if not codepoint or codepoint > 0x10FFFF:
            return ""
        character = chr(codepoint)
        if caps and character.isalpha():
            character = character.lower() if shift else character.upper()
        return character if character.isprintable() or character in "\t\n" else ""

    def _key_name(self, keycode: int) -> str:
        if not self._control:
            return ""
        keysym = self._libraries.x11.XkbKeycodeToKeysym(self._control, keycode, 0, 0)
        name = self._libraries.x11.XKeysymToString(keysym)
        return name.decode("ascii", "replace") if name else ""

    def _consume_expected(self, pressed: bool, keycode: int) -> bool:
        with self._expected_lock:
            now = time.monotonic()
            if self._expected and now > self._expected_deadline:
                self._expected.clear()
            if self._expected and self._expected[0] == (pressed, keycode):
                self._expected.popleft()
                return True
            return False

    def current_group(self) -> int:
        if not self._control:
            return -1
        state = XkbStateRec()
        result = self._libraries.x11.XkbGetState(
            self._control, XKB_USE_CORE_KBD, ctypes.byref(state)
        )
        return int(state.group) if result == 0 else -1

    def input_anchor(self) -> ScreenAnchor | None:
        if not self._control:
            return None
        screen = int(self._libraries.x11.XDefaultScreen(self._control))
        root = int(self._libraries.x11.XRootWindow(self._control, screen))
        root_return = ctypes.c_ulong()
        child_return = ctypes.c_ulong()
        root_x = ctypes.c_int()
        root_y = ctypes.c_int()
        window_x = ctypes.c_int()
        window_y = ctypes.c_int()
        mask = ctypes.c_uint()
        if not self._libraries.x11.XQueryPointer(
            self._control,
            root,
            ctypes.byref(root_return),
            ctypes.byref(child_return),
            ctypes.byref(root_x),
            ctypes.byref(root_y),
            ctypes.byref(window_x),
            ctypes.byref(window_y),
            ctypes.byref(mask),
        ):
            return None
        focus = ctypes.c_ulong()
        revert = ctypes.c_int()
        self._libraries.x11.XGetInputFocus(
            self._control, ctypes.byref(focus), ctypes.byref(revert)
        )
        return ScreenAnchor(root_x.value, root_y.value, int(focus.value) or None)

    def focused_window(self) -> FocusInfo | None:
        if not self._control:
            return None
        window, revert = ctypes.c_ulong(), ctypes.c_int()
        if not self._libraries.x11.XGetInputFocus(
            self._control, ctypes.byref(window), ctypes.byref(revert)
        ):
            return None
        focus = int(window.value)
        if focus in (0, 1):  # None or PointerRoot: nobody has the focus
            return None
        return FocusInfo(focus, self._window_is_own(focus))

    def _window_is_own(self, window: int) -> bool:
        cached = self._own_windows.get(window)
        if cached is not None:
            return cached
        own = self._window_process_id(window) == os.getpid()
        if len(self._own_windows) >= 256:
            self._own_windows.clear()
        self._own_windows[window] = own
        return own

    def window_process_id(self, window: int) -> int:
        """Resolve the exact client process for optional accessibility reads."""
        return self._window_process_id(window)

    def _window_process_id(self, window: int) -> int:
        """``_NET_WM_PID`` of the client owning ``window`` (GTK sets it).

        Toolkits give the focus to a child window that carries no properties
        of its own, so the search walks up a bounded number of parents.
        """

        if self._net_wm_pid_atom is None:
            self._net_wm_pid_atom = int(
                self._libraries.x11.XInternAtom(self._control, b"_NET_WM_PID", 1)
            )
        atom = self._net_wm_pid_atom
        if not atom:
            return 0
        current = window
        for _ in range(16):
            process_id = self._cardinal_property(current, atom)
            if process_id:
                return process_id
            parent = self._parent_window(current)
            if parent in (0, current):
                return 0
            current = parent
        return 0

    def _cardinal_property(self, window: int, atom: int) -> int:
        actual_type, actual_format = ctypes.c_ulong(), ctypes.c_int()
        count, remaining = ctypes.c_ulong(), ctypes.c_ulong()
        data = ctypes.c_void_p()
        status = self._libraries.x11.XGetWindowProperty(
            self._control,
            window,
            atom,
            0,
            1,
            0,
            XA_CARDINAL,
            ctypes.byref(actual_type),
            ctypes.byref(actual_format),
            ctypes.byref(count),
            ctypes.byref(remaining),
            ctypes.byref(data),
        )
        if status != 0 or not data.value:
            return 0
        try:
            if actual_format.value != 32 or count.value < 1:
                return 0
            return int(ctypes.cast(data, ctypes.POINTER(ctypes.c_ulong))[0])
        finally:
            self._libraries.x11.XFree(data)

    def _parent_window(self, window: int) -> int:
        root, parent = ctypes.c_ulong(), ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        count = ctypes.c_uint()
        if not self._libraries.x11.XQueryTree(
            self._control,
            window,
            ctypes.byref(root),
            ctypes.byref(parent),
            ctypes.byref(children),
            ctypes.byref(count),
        ):
            return 0
        if ctypes.cast(children, ctypes.c_void_p).value is not None:
            self._libraries.x11.XFree(children)
        return int(parent.value)

    def position_window(self, window: int, x: int, y: int) -> bool:
        if not self._control or not window:
            return False
        self._libraries.x11.XMoveWindow(self._control, window, x, y)
        self._libraries.x11.XRaiseWindow(self._control, window)
        self._libraries.x11.XFlush(self._control)
        return True

    def restore_window(self, window: int | None) -> bool:
        if not self._control or window is None:
            return False
        self._libraries.x11.XSetInputFocus(
            self._control, window, REVERT_TO_PARENT, CURRENT_TIME
        )
        self._libraries.x11.XFlush(self._control)
        return True

    def switch_group(self, group: int) -> None:
        if not 0 <= group < self.group_count:
            raise X11Error(f"Неизвестная группа раскладки {group}")
        if not self._control:
            raise X11Error("X11 backend не запущен")
        with self._inject_lock:
            if not self._libraries.x11.XkbLockGroup(
                self._control, XKB_USE_CORE_KBD, group
            ):
                raise X11Error(f"Не удалось переключить XKB-группу на {group}")
            self._libraries.x11.XFlush(self._control)

    def hold_input(self) -> None:
        """Accepted for the shared contract; XRecord cannot withhold keys."""

        return None

    def release_input(self) -> int:
        return 0

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
    ) -> int:
        if not self._control:
            raise X11Error("X11 backend не запущен")
        stroke_list = list(strokes)
        late_list = list(late)
        keyboard_state = XkbStateRec()
        if self._libraries.x11.XkbGetState(self._control, XKB_USE_CORE_KBD, ctypes.byref(keyboard_state)) != 0:
            raise X11Error("Не удалось прочитать состояние Caps Lock перед заменой")
        caps_lock = bool(keyboard_state.locked_mods & LOCK_MASK)

        def shifted(stroke: KeyEvent, group: int) -> bool:
            return stroke.shift ^ (
                stroke.character_for(group).isalpha() and stroke.caps_lock != caps_lock
            )
        backspace_keycode = int(self._libraries.x11.XKeysymToKeycode(self._control, 0xFF08))
        shift_keycode = int(self._libraries.x11.XKeysymToKeycode(self._control, 0xFFE1))
        if not backspace_keycode or not shift_keycode:
            raise X11Error("X-сервер не вернул keycode для BackSpace/Shift")
        sequence: list[tuple[bool, int]] = []

        def tap(
            target: list[tuple[bool, int]], keycode: int, shifted: bool = False
        ) -> None:
            if shifted:
                target.append((True, shift_keycode))
            target.extend(((True, keycode), (False, keycode)))
            if shifted:
                target.append((False, shift_keycode))

        delete_count = (
            len(stroke_list) + (1 if boundary is not None else 0) + len(late_list)
        )
        for _ in range(delete_count):
            tap(sequence, backspace_keycode)
        for stroke in stroke_list:
            tap(sequence, stroke.keycode, shifted(stroke, target_group))
        # Keys typed after the word are deleted with it and typed again in the
        # new layout. They are left out of ``_expected`` on purpose: the engine
        # must see them come back as the user's own input.
        late_sequence: list[tuple[bool, int]] = []
        for stroke in late_list:
            tap(late_sequence, stroke.keycode, shifted(stroke, target_group))
        rendered_source_group = (
            source_group
            if source_group is not None
            else stroke_list[0].group if stroke_list else target_group
        )
        preserve_boundary_layout = bool(
            boundary is not None
            and rendered_source_group != target_group
            and boundary.character
            and boundary.character_for(target_group) != boundary.character
        )
        # Keep boundary events immutable and inject them separately. This also
        # avoids a Nuitka 4.1/Python 3.14 list-mutation compiler regression.
        boundary_sequence: tuple[tuple[bool, int], ...] = ()
        if boundary is not None and boundary.shift:
            boundary_sequence = (
                (True, shift_keycode),
                (True, boundary.keycode),
                (False, boundary.keycode),
                (False, shift_keycode),
            )
        elif boundary is not None:
            boundary_sequence = (
                (True, boundary.keycode),
                (False, boundary.keycode),
            )
        target_boundary_sequence = (
            () if preserve_boundary_layout else boundary_sequence
        )
        source_boundary_sequence = (
            boundary_sequence if preserve_boundary_layout else ()
        )
        expected_count = (
            len(sequence)
            + len(target_boundary_sequence)
            + len(source_boundary_sequence)
        )
        with self._inject_lock:
            with self._expected_lock:
                self._expected.extend(sequence)
                self._expected.extend(target_boundary_sequence)
                self._expected.extend(source_boundary_sequence)
                self._expected_deadline = time.monotonic() + max(
                    1.0, expected_count * 0.02
                )
            self._libraries.xtst.XTestGrabControl(self._control, 1)
            try:
                if not self._libraries.x11.XkbLockGroup(
                    self._control, XKB_USE_CORE_KBD, target_group
                ):
                    raise X11Error(f"Не удалось включить XKB-группу {target_group}")
                for pressed, keycode in sequence:
                    if not self._libraries.xtst.XTestFakeKeyEvent(
                        self._control, keycode, int(pressed), 0
                    ):
                        raise X11Error(f"XTest отклонил keycode {keycode}")
                for pressed, keycode in target_boundary_sequence:
                    if not self._libraries.xtst.XTestFakeKeyEvent(
                        self._control, keycode, int(pressed), 0
                    ):
                        raise X11Error(f"XTest отклонил keycode {keycode}")
                if source_boundary_sequence:
                    if not self._libraries.x11.XkbLockGroup(
                        self._control, XKB_USE_CORE_KBD, rendered_source_group
                    ):
                        raise X11Error(
                            "Не удалось временно включить "
                            f"XKB-группу {rendered_source_group}"
                        )
                    for pressed, keycode in source_boundary_sequence:
                        if not self._libraries.xtst.XTestFakeKeyEvent(
                            self._control, keycode, int(pressed), 0
                        ):
                            raise X11Error(f"XTest отклонил keycode {keycode}")
                    if not self._libraries.x11.XkbLockGroup(
                        self._control, XKB_USE_CORE_KBD, target_group
                    ):
                        raise X11Error(
                            f"Не удалось восстановить XKB-группу {target_group}"
                        )
                for pressed, keycode in late_sequence:
                    if not self._libraries.xtst.XTestFakeKeyEvent(
                        self._control, keycode, int(pressed), 0
                    ):
                        raise X11Error(f"XTest отклонил keycode {keycode}")
                self._libraries.x11.XSync(self._control, 0)
            except Exception:
                with self._expected_lock:
                    self._expected.clear()
                raise
            finally:
                self._libraries.xtst.XTestGrabControl(self._control, 0)
                self._libraries.x11.XFlush(self._control)
        return 0

    def active_application(self) -> str:
        if not self._control:
            return ""
        window, revert = ctypes.c_ulong(), ctypes.c_int()
        if not self._libraries.x11.XGetInputFocus(
            self._control, ctypes.byref(window), ctypes.byref(revert)
        ):
            return ""
        current = window.value
        for _ in range(16):
            if not current:
                break
            hint = XClassHint()
            if self._libraries.x11.XGetClassHint(self._control, current, ctypes.byref(hint)):
                try:
                    class_name = ctypes.string_at(hint.res_class).decode("utf-8", "replace") if hint.res_class else ""
                    resource_name = ctypes.string_at(hint.res_name).decode("utf-8", "replace") if hint.res_name else ""
                    return class_name or resource_name
                finally:
                    if hint.res_name:
                        self._libraries.x11.XFree(hint.res_name)
                    if hint.res_class:
                        self._libraries.x11.XFree(hint.res_class)
            root, parent = ctypes.c_ulong(), ctypes.c_ulong()
            children = ctypes.POINTER(ctypes.c_ulong)()
            count = ctypes.c_uint()
            if not self._libraries.x11.XQueryTree(
                self._control,
                current,
                ctypes.byref(root),
                ctypes.byref(parent),
                ctypes.byref(children),
                ctypes.byref(count),
            ):
                break
            if ctypes.cast(children, ctypes.c_void_p).value is not None:
                self._libraries.x11.XFree(children)
            if not parent.value or parent.value == current:
                break
            current = parent.value
        return ""

    def stop(self) -> None:
        if self._running.is_set() and self._control and self._context:
            self._running.clear()
            self._libraries.xtst.XRecordDisableContext(self._control, self._context)
            self._libraries.x11.XSync(self._control, 0)
        if self._thread and self._thread is not threading.current_thread():
            self._thread.join(timeout=2.0)
        self._thread = None

    def close(self) -> None:
        self.stop()
        if self._context and self._control:
            self._libraries.xtst.XRecordFreeContext(self._control, self._context)
            self._context = 0
        if self._range:
            self._libraries.x11.XFree(self._range)
            self._range = None
        if self._record:
            self._libraries.x11.XCloseDisplay(self._record)
            self._record = None
        if self._control:
            self._libraries.x11.XCloseDisplay(self._control)
            self._control = None

    def __enter__(self) -> "X11Backend":
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()
