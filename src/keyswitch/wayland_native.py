"""Linux kernel evdev/uinput, Wayland/XKB ctypes and desktop D-Bus/IPC glue.

All pure parsing, device classification, binary framing and backend decision logic
live in ``wayland_desktop``, ``wayland_helper`` and ``wayland_backend``; this module
binds them to live Linux file descriptors, ``libwayland-client``, ``libxkbcommon``,
``Gio.DBus`` and compositor sockets.
"""

from __future__ import annotations

from collections.abc import Callable, Sequence
import ctypes
import fcntl
import json
import logging
import mmap
import os
from pathlib import Path
import select
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from typing import Protocol, cast

import gi

gi.require_version("Gio", "2.0")
gi.require_version("GLib", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

from .backend import ScreenAnchor
from .constants.file_formats import HEXADECIMAL_BASE
from .constants.keyboard import LOCK_MASK, SHIFT_MASK
from .constants.learning_prompt import LEARNING_PROMPT_WINDOW_TITLE
from .constants.timing import (
    LAYOUT_SWITCH_POLL_SECONDS,
    WAYLAND_DBUS_CALL_TIMEOUT_MS,
    WAYLAND_HELPER_RESTART_DELAY_SECONDS,
    WAYLAND_IPC_TIMEOUT_SECONDS,
    WAYLAND_KEYMAP_WAIT_SECONDS,
    WAYLAND_REPEAT_DEFAULT_DELAY_MS,
)
from .constants.units import BITS_PER_BYTE, MILLISECONDS_PER_SECOND
from .constants.wayland import (
    BUS_I8042,
    EV_LED,
    EV_KEY,
    EV_SYN,
    EVDEV_KEY_BITMAP_BYTES,
    EVDEV_NAME_BUFFER_BYTES,
    EVDEV_READ_BATCH_EVENTS,
    EVDEV_SMALL_BITMAP_BYTES,
    EVIOCGBIT_ABS_8,
    EVIOCGBIT_EV_8,
    EVIOCGBIT_KEY_96,
    EVIOCGBIT_REL_8,
    EVIOCGID,
    EVIOCGKEY_96,
    EVIOCGLED_8,
    EVIOCGNAME_256,
    EVIOCGPHYS_256,
    EVIOCGRAB,
    EVIOCSCLOCKID,
    IN_ATTRIB,
    IN_CLOEXEC,
    IN_CREATE,
    IN_DELETE,
    IN_NONBLOCK,
    INOTIFY_EVENT_HEADER_BYTES,
    INOTIFY_READ_BUFFER_BYTES,
    LED_CAPSL,
    SWAY_IPC_EVENT_INPUT,
    SWAY_IPC_EVENT_WINDOW,
    SWAY_IPC_GET_INPUTS,
    SWAY_IPC_GET_TREE,
    SWAY_IPC_HEADER_BYTES,
    SWAY_IPC_READ_CHUNK_BYTES,
    SWAY_IPC_RUN_COMMAND,
    SWAY_IPC_SUBSCRIBE,
    UI_DEV_CREATE,
    UI_DEV_DESTROY,
    UI_DEV_SETUP,
    UI_SET_EVBIT,
    UI_SET_KEYBIT,
    UI_SET_LEDBIT,
    UINT32_MODULO_MASK,
    UINPUT_MAX_NAME_BYTES,
    WAYLAND_FORWARDED_LEDS,
    WAYLAND_HELPER_RESTARTS,
    WAYLAND_KEYMAP_ROUNDTRIPS,
    WAYLAND_SEAT_INTERFACE_MAX_VERSION,
    WAYLAND_UINPUT_MAX_KEYCODE,
    WAYLAND_UINPUT_PRODUCT_ID,
    WAYLAND_UINPUT_VENDOR_ID,
    WAYLAND_UINPUT_VERSION,
    WAYLAND_XKB_KEYCODE_OFFSET,
    WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1,
    WL_SEAT_CAPABILITY_KEYBOARD,
    XKB_KEYSYM_NAME_BUFFER_BYTES,
)
from .wayland_backend import NativeKeyEvent
from .wayland_desktop import (
    DesktopSnapshot,
    DesktopWindowInfo,
    GNOME_DBUS_INTERFACE,
    GNOME_DBUS_NAME,
    GNOME_DBUS_PATH,
    GNOME_EXTENSION_UUID,
    GNOME_EXTENSIONS_BUS_NAME,
    GNOME_EXTENSIONS_INTERFACE,
    GNOME_EXTENSIONS_PATH,
    KDE_KEYBOARD_BUS_NAME,
    KDE_KEYBOARD_INTERFACE,
    KDE_KEYBOARD_PATH,
    KWIN_BUS_NAME,
    KWIN_SCRIPT_INTERFACE,
    KWIN_SCRIPTING_INTERFACE,
    KWIN_SCRIPTING_PATH,
    KdeWindowIdMap,
    build_kwin_action_script,
    build_kwin_watcher_script,
    decode_sway_ipc_header,
    detect_desktop_kind,
    encode_sway_ipc,
    find_sway_window_by_pid,
    parse_gnome_state,
    parse_kde_layouts,
    parse_kwin_report,
    parse_sway_focused_window,
    parse_sway_inputs,
    sway_focus_command,
    sway_popup_move_command,
    sway_switch_layout_command,
)
from .wayland_helper import (
    HelperClient,
    HelperKey,
    InputDeviceInfo,
    RawInputEvent,
    WaylandInputHelper,
    classify_input_device,
)

LOGGER = logging.getLogger(__name__)

DEV_INPUT_DIR = Path("/dev/input")
SYS_INPUT_DIR = Path("/sys/class/input")
UINPUT_PATH = "/dev/uinput"
VIRTUAL_KEYBOARD_NAME = b"KeySwitch Virtual Keyboard"

KWIN_CALLBACK_PATH = "/io/github/olegius88/KeySwitch/KWin"
KWIN_CALLBACK_IFACE = "io.github.olegius88.KeySwitch.KWin"
KWIN_WATCHER_PLUGIN = "keyswitch_wayland_watcher"
KWIN_ACTION_PLUGIN = "keyswitch_wayland_action"
KWIN_CALLBACK_XML = (
    "<node>"
    f'<interface name="{KWIN_CALLBACK_IFACE}">'
    '<method name="Report"><arg type="s" name="payload" direction="in"/></method>'
    "</interface>"
    "</node>"
)

LOGIND_BUS_NAME = "org.freedesktop.login1"
LOGIND_MANAGER_PATH = "/org/freedesktop/login1"
LOGIND_USER_SELF_PATH = "/org/freedesktop/login1/user/self"
LOGIND_MANAGER_IFACE = "org.freedesktop.login1.Manager"
LOGIND_USER_IFACE = "org.freedesktop.login1.User"
LOGIND_SESSION_IFACE = "org.freedesktop.login1.Session"
DBUS_PROPERTIES_IFACE = "org.freedesktop.DBus.Properties"

INPUT_EVENT_STRUCT = struct.Struct("@llHHi")
INPUT_ID_STRUCT = struct.Struct("@HHHH")
UINPUT_SETUP_STRUCT = struct.Struct("@HHHH80sI")
INOTIFY_HEADER_STRUCT = struct.Struct("=iIII")
INT_STRUCT = struct.Struct("@i")


def _bits_from_bytes(buf: bytes) -> frozenset[int]:
    out: set[int] = set()
    for byte_idx, val in enumerate(buf):
        if not val:
            continue
        base = byte_idx * BITS_PER_BYTE
        for bit in range(BITS_PER_BYTE):
            if (val >> bit) & 1:
                out.add(base + bit)
    return frozenset(out)


def _bits_from_sysfs(raw: str) -> frozenset[int]:
    tokens = raw.strip().split()
    if not tokens:
        return frozenset()
    word_bits = ctypes.sizeof(ctypes.c_ulong) * BITS_PER_BYTE
    out: set[int] = set()
    for word_idx, token in enumerate(reversed(tokens)):
        try:
            word = int(token, HEXADECIMAL_BASE)
        except ValueError:
            continue
        if not word:
            continue
        base = word_idx * word_bits
        for bit in range(word_bits):
            if (word >> bit) & 1:
                out.add(base + bit)
    return frozenset(out)


def _read_sysfs_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return ""


def _read_sysfs_hex(path: Path) -> int:
    raw = _read_sysfs_text(path)
    if not raw:
        return 0
    try:
        return int(raw, HEXADECIMAL_BASE)
    except ValueError:
        return 0


class LinuxHelperOS:
    """Real Linux ``evdev`` / ``uinput`` / ``inotify`` / ``poll`` implementation."""

    def __init__(self) -> None:
        self._libc = ctypes.CDLL("libc.so.6", use_errno=True)
        self._libc.inotify_init1.argtypes = [ctypes.c_int]
        self._libc.inotify_init1.restype = ctypes.c_int
        self._libc.inotify_add_watch.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint32,
        ]
        self._libc.inotify_add_watch.restype = ctypes.c_int

    def create_uinput(self) -> int:
        fd = os.open(UINPUT_PATH, os.O_RDWR | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            fcntl.ioctl(fd, UI_SET_EVBIT, EV_SYN)
            fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
            for code in range(1, WAYLAND_UINPUT_MAX_KEYCODE + 1):
                fcntl.ioctl(fd, UI_SET_KEYBIT, code)
            fcntl.ioctl(fd, UI_SET_EVBIT, EV_LED)
            for led in WAYLAND_FORWARDED_LEDS:
                fcntl.ioctl(fd, UI_SET_LEDBIT, led)
            setup = UINPUT_SETUP_STRUCT.pack(
                BUS_I8042,
                WAYLAND_UINPUT_VENDOR_ID,
                WAYLAND_UINPUT_PRODUCT_ID,
                WAYLAND_UINPUT_VERSION,
                VIRTUAL_KEYBOARD_NAME.ljust(UINPUT_MAX_NAME_BYTES, b"\x00"),
                0,
            )
            fcntl.ioctl(fd, UI_DEV_SETUP, setup)
            fcntl.ioctl(fd, UI_DEV_CREATE)
            return fd
        except Exception:
            os.close(fd)
            raise

    def destroy_uinput(self, fd: int) -> None:
        try:
            fcntl.ioctl(fd, UI_DEV_DESTROY)
        finally:
            os.close(fd)

    def write_uinput(self, fd: int, events: Sequence[tuple[int, int, int]]) -> None:
        payload = b"".join(
            INPUT_EVENT_STRUCT.pack(0, 0, ev_type, code, value)
            for ev_type, code, value in events
        )
        os.write(fd, payload)

    def read_uinput_leds(self, fd: int) -> Sequence[tuple[int, int]]:
        leds: list[tuple[int, int]] = []
        size = INPUT_EVENT_STRUCT.size
        while True:
            try:
                chunk = os.read(fd, size * EVDEV_READ_BATCH_EVENTS)
            except BlockingIOError:
                break
            if not chunk:
                break
            count = len(chunk) // size
            for idx in range(count):
                _sec, _usec, ev_type, code, value = INPUT_EVENT_STRUCT.unpack_from(
                    chunk, idx * size
                )
                if ev_type == EV_LED:
                    leds.append((int(code), int(value)))
        return leds

    def list_event_devices(self) -> Sequence[InputDeviceInfo]:
        if not DEV_INPUT_DIR.is_dir():
            return ()
        results: list[InputDeviceInfo] = []
        for node in sorted(DEV_INPUT_DIR.glob("event*")):
            info = self._inspect_sysfs(node) or self._inspect_ioctl(node)
            if info is not None:
                results.append(info)
        return results

    def _inspect_sysfs(self, node: Path) -> InputDeviceInfo | None:
        sys_dev = SYS_INPUT_DIR / node.name / "device"
        if not sys_dev.is_dir():
            return None
        ev_raw = _read_sysfs_text(sys_dev / "capabilities" / "ev")
        if not ev_raw:
            return None
        return InputDeviceInfo(
            path=str(node),
            name=_read_sysfs_text(sys_dev / "name"),
            phys=_read_sysfs_text(sys_dev / "phys"),
            bustype=_read_sysfs_hex(sys_dev / "id" / "bustype"),
            vendor=_read_sysfs_hex(sys_dev / "id" / "vendor"),
            product=_read_sysfs_hex(sys_dev / "id" / "product"),
            ev_bits=_bits_from_sysfs(ev_raw),
            key_bits=_bits_from_sysfs(_read_sysfs_text(sys_dev / "capabilities" / "key")),
            rel_bits=_bits_from_sysfs(_read_sysfs_text(sys_dev / "capabilities" / "rel")),
            abs_bits=_bits_from_sysfs(_read_sysfs_text(sys_dev / "capabilities" / "abs")),
        )

    def _inspect_ioctl(self, node: Path) -> InputDeviceInfo | None:
        try:
            fd = os.open(str(node), os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
        except OSError:
            return None
        try:
            id_buf = bytearray(INPUT_ID_STRUCT.size)
            fcntl.ioctl(fd, EVIOCGID, id_buf)
            bustype, vendor, product, _version = INPUT_ID_STRUCT.unpack(id_buf)
            name_buf = bytearray(EVDEV_NAME_BUFFER_BYTES)
            try:
                fcntl.ioctl(fd, EVIOCGNAME_256, name_buf)
            except OSError:
                pass
            phys_buf = bytearray(EVDEV_NAME_BUFFER_BYTES)
            try:
                fcntl.ioctl(fd, EVIOCGPHYS_256, phys_buf)
            except OSError:
                pass
            ev_buf = bytearray(EVDEV_SMALL_BITMAP_BYTES)
            key_buf = bytearray(EVDEV_KEY_BITMAP_BYTES)
            rel_buf = bytearray(EVDEV_SMALL_BITMAP_BYTES)
            abs_buf = bytearray(EVDEV_SMALL_BITMAP_BYTES)
            for req, buf in (
                (EVIOCGBIT_EV_8, ev_buf),
                (EVIOCGBIT_KEY_96, key_buf),
                (EVIOCGBIT_REL_8, rel_buf),
                (EVIOCGBIT_ABS_8, abs_buf),
            ):
                try:
                    fcntl.ioctl(fd, req, buf)
                except OSError:
                    pass
            return InputDeviceInfo(
                path=str(node),
                name=bytes(name_buf).split(b"\x00", 1)[0].decode("utf-8", errors="replace"),
                phys=bytes(phys_buf).split(b"\x00", 1)[0].decode("utf-8", errors="replace"),
                bustype=int(bustype),
                vendor=int(vendor),
                product=int(product),
                ev_bits=_bits_from_bytes(bytes(ev_buf)),
                key_bits=_bits_from_bytes(bytes(key_buf)),
                rel_bits=_bits_from_bytes(bytes(rel_buf)),
                abs_bits=_bits_from_bytes(bytes(abs_buf)),
            )
        except OSError:
            return None
        finally:
            os.close(fd)

    def open_device(self, path: str, *, writable: bool) -> int:
        flags = (os.O_RDWR if writable else os.O_RDONLY) | os.O_NONBLOCK | os.O_CLOEXEC
        fd = os.open(path, flags)
        try:
            fcntl.ioctl(fd, EVIOCSCLOCKID, INT_STRUCT.pack(time.CLOCK_MONOTONIC))
        except OSError:
            pass
        return fd

    def close_device(self, fd: int) -> None:
        os.close(fd)

    def grab_device(self, fd: int) -> None:
        fcntl.ioctl(fd, EVIOCGRAB, 1)

    def ungrab_device(self, fd: int) -> None:
        fcntl.ioctl(fd, EVIOCGRAB, 0)

    def query_keys_down(self, fd: int) -> frozenset[int]:
        buf = bytearray(EVDEV_KEY_BITMAP_BYTES)
        fcntl.ioctl(fd, EVIOCGKEY_96, buf)
        return _bits_from_bytes(bytes(buf))

    def query_caps_lock(self, fd: int) -> bool:
        buf = bytearray(EVDEV_SMALL_BITMAP_BYTES)
        try:
            fcntl.ioctl(fd, EVIOCGLED_8, buf)
        except OSError:
            return False
        return LED_CAPSL in _bits_from_bytes(bytes(buf))

    def read_events(self, fd: int) -> Sequence[RawInputEvent]:
        events: list[RawInputEvent] = []
        size = INPUT_EVENT_STRUCT.size
        while True:
            try:
                chunk = os.read(fd, size * EVDEV_READ_BATCH_EVENTS)
            except BlockingIOError:
                break
            if not chunk:
                break
            count = len(chunk) // size
            for idx in range(count):
                sec, usec, ev_type, code, value = INPUT_EVENT_STRUCT.unpack_from(
                    chunk, idx * size
                )
                events.append(
                    RawInputEvent(int(sec), int(usec), int(ev_type), int(code), int(value))
                )
        return events

    def write_led(self, fd: int, code: int, value: int) -> None:
        os.write(fd, INPUT_EVENT_STRUCT.pack(0, 0, EV_LED, code, value))

    def open_inotify(self) -> int:
        fd = int(self._libc.inotify_init1(IN_NONBLOCK | IN_CLOEXEC))
        if fd < 0:
            errno_val = ctypes.get_errno()
            raise OSError(errno_val, os.strerror(errno_val))
        mask = IN_CREATE | IN_ATTRIB | IN_DELETE
        wd = int(self._libc.inotify_add_watch(fd, b"/dev/input", mask))
        if wd < 0:
            errno_val = ctypes.get_errno()
            os.close(fd)
            raise OSError(errno_val, os.strerror(errno_val))
        return fd

    def read_inotify(self, fd: int) -> Sequence[str]:
        names: list[str] = []
        while True:
            try:
                buf = os.read(fd, INOTIFY_READ_BUFFER_BYTES)
            except BlockingIOError:
                break
            if not buf:
                break
            offset = 0
            while offset + INOTIFY_EVENT_HEADER_BYTES <= len(buf):
                _wd, _mask, _cookie, length = INOTIFY_HEADER_STRUCT.unpack_from(buf, offset)
                offset += INOTIFY_EVENT_HEADER_BYTES
                raw_name = buf[offset : offset + length].split(b"\x00", 1)[0]
                offset += length
                if raw_name:
                    names.append(raw_name.decode("utf-8", errors="replace"))
        return names

    def poll(self, fds: Sequence[int], timeout_ms: int) -> Sequence[tuple[int, int]]:
        poller = select.poll()
        for fd in fds:
            poller.register(fd, select.POLLIN | select.POLLHUP | select.POLLERR)
        return poller.poll(timeout_ms)

    def monotonic(self) -> float:
        return time.monotonic()

    def sleep(self, seconds: float) -> None:
        time.sleep(seconds)


def run_wayland_input_helper(sock_fd: int) -> int:
    """Entry point for ``keyswitch --wayland-input-helper <fd>``."""

    sock = socket.socket(fileno=sock_fd)
    sock.setblocking(True)
    helper = WaylandInputHelper(sock, LinuxHelperOS())

    def _request_stop(_signum: int, _frame: object) -> None:
        helper.stop()

    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, _request_stop)
    if hasattr(signal, "SIGTSTP"):
        signal.signal(signal.SIGTSTP, signal.SIG_IGN)
    try:
        return helper.run()
    finally:
        try:
            sock.close()
        except OSError:
            pass


# --- Wayland wl_keyboard + libxkbcommon monitor ---


class _WlMessage(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("name", ctypes.c_char_p),
        ("signature", ctypes.c_char_p),
        ("types", ctypes.c_void_p),
    ]


class _WlInterface(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("name", ctypes.c_char_p),
        ("version", ctypes.c_int),
        ("method_count", ctypes.c_int),
        ("methods", ctypes.POINTER(_WlMessage)),
        ("event_count", ctypes.c_int),
        ("events", ctypes.POINTER(_WlMessage)),
    ]


_RegGlobalFn = ctypes.CFUNCTYPE(
    None,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint32,
    ctypes.c_char_p,
    ctypes.c_uint32,
)
_RegRemoveFn = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32)


class _WlRegistryListener(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("global_cb", _RegGlobalFn),
        ("global_remove_cb", _RegRemoveFn),
    ]


_SeatCapabilitiesFn = ctypes.CFUNCTYPE(
    None, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32
)
_SeatNameFn = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_char_p)


class _WlSeatListener(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("capabilities", _SeatCapabilitiesFn),
        ("name", _SeatNameFn),
    ]


_KbdKeymapFn = ctypes.CFUNCTYPE(
    None,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint32,
    ctypes.c_int32,
    ctypes.c_uint32,
)
_KbdEnterFn = ctypes.CFUNCTYPE(
    None,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint32,
    ctypes.c_void_p,
    ctypes.c_void_p,
)
_KbdLeaveFn = ctypes.CFUNCTYPE(
    None, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p
)
_KbdKeyFn = ctypes.CFUNCTYPE(
    None,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_uint32,
)
_KbdModifiersFn = ctypes.CFUNCTYPE(
    None,
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_uint32,
)
_KbdRepeatInfoFn = ctypes.CFUNCTYPE(
    None, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int32, ctypes.c_int32
)


class _WlKeyboardListener(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("keymap", _KbdKeymapFn),
        ("enter", _KbdEnterFn),
        ("leave", _KbdLeaveFn),
        ("key", _KbdKeyFn),
        ("modifiers", _KbdModifiersFn),
        ("repeat_info", _KbdRepeatInfoFn),
    ]


class _XkbRuleNames(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("rules", ctypes.c_char_p),
        ("model", ctypes.c_char_p),
        ("layout", ctypes.c_char_p),
        ("variant", ctypes.c_char_p),
        ("options", ctypes.c_char_p),
    ]


_MarshalNewObjFn = ctypes.CFUNCTYPE(
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint32,
    ctypes.POINTER(_WlInterface),
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_void_p,
)
_MarshalBindFn = ctypes.CFUNCTYPE(
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_uint32,
    ctypes.POINTER(_WlInterface),
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_uint32,
    ctypes.c_char_p,
    ctypes.c_uint32,
    ctypes.c_void_p,
)


class WaylandXkbMonitor:
    """Connects to ``WAYLAND_DISPLAY`` to track the live ``wl_keyboard`` XKB keymap."""

    def __init__(self, on_repeat_delay: Callable[[int], None] | None = None) -> None:
        self._on_repeat_delay = on_repeat_delay
        self._xkb = ctypes.CDLL("libxkbcommon.so.0")
        self._configure_xkb()
        self._xkb_ctx = self._xkb.xkb_context_new(0)
        if not self._xkb_ctx:
            raise RuntimeError("Не удалось инициализировать libxkbcommon")
        self._lock = threading.Lock()
        self._keymap: int | None = None
        self._state: int | None = None
        self._shift_mod = -1
        self._lock_mod = -1
        self._layout_names: tuple[str, ...] = ()
        # Answers of the keymap, kept until it changes: every key the helper asks about is translated
        # for each layout, and each ctypes call gives the interpreter lock away while the engine's
        # worker holds it for the model, which delayed the helper's answer past its timeout.
        self._translations: dict[tuple[int, int, int], str] = {}
        self._symbols: dict[tuple[int, int], str] = {}
        self._active_group = 0
        self._repeat_delay_ms = WAYLAND_REPEAT_DEFAULT_DELAY_MS
        self._keymap_ready = threading.Event()
        self._stop_event = threading.Event()
        self._wake_r: int | None = None
        self._wake_w: int | None = None
        self._thread: threading.Thread | None = None

    def _configure_xkb(self) -> None:
        self._xkb.xkb_context_new.argtypes = [ctypes.c_int]
        self._xkb.xkb_context_new.restype = ctypes.c_void_p
        self._xkb.xkb_context_unref.argtypes = [ctypes.c_void_p]
        self._xkb.xkb_context_unref.restype = None
        self._xkb.xkb_keymap_new_from_string.argtypes = [
            ctypes.c_void_p,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_int,
        ]
        self._xkb.xkb_keymap_new_from_string.restype = ctypes.c_void_p
        self._xkb.xkb_keymap_new_from_names.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(_XkbRuleNames),
            ctypes.c_int,
        ]
        self._xkb.xkb_keymap_new_from_names.restype = ctypes.c_void_p
        self._xkb.xkb_keymap_unref.argtypes = [ctypes.c_void_p]
        self._xkb.xkb_keymap_unref.restype = None
        self._xkb.xkb_keymap_num_layouts.argtypes = [ctypes.c_void_p]
        self._xkb.xkb_keymap_num_layouts.restype = ctypes.c_uint32
        self._xkb.xkb_keymap_layout_get_name.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        self._xkb.xkb_keymap_layout_get_name.restype = ctypes.c_char_p
        self._xkb.xkb_keymap_mod_get_index.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        self._xkb.xkb_keymap_mod_get_index.restype = ctypes.c_uint32
        self._xkb.xkb_state_new.argtypes = [ctypes.c_void_p]
        self._xkb.xkb_state_new.restype = ctypes.c_void_p
        self._xkb.xkb_state_unref.argtypes = [ctypes.c_void_p]
        self._xkb.xkb_state_unref.restype = None
        self._xkb.xkb_state_update_mask.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint32,
            ctypes.c_uint32,
            ctypes.c_uint32,
            ctypes.c_uint32,
            ctypes.c_uint32,
            ctypes.c_uint32,
        ]
        self._xkb.xkb_state_update_mask.restype = ctypes.c_int
        self._xkb.xkb_state_key_get_utf32.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        self._xkb.xkb_state_key_get_utf32.restype = ctypes.c_uint32
        self._xkb.xkb_state_key_get_one_sym.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        self._xkb.xkb_state_key_get_one_sym.restype = ctypes.c_uint32
        self._xkb.xkb_keysym_get_name.argtypes = [
            ctypes.c_uint32,
            ctypes.c_char_p,
            ctypes.c_size_t,
        ]
        self._xkb.xkb_keysym_get_name.restype = ctypes.c_int

    @property
    def layout_names(self) -> tuple[str, ...]:
        with self._lock:
            return self._layout_names

    @property
    def active_group(self) -> int:
        return self._active_group

    @property
    def repeat_delay_ms(self) -> int:
        return self._repeat_delay_ms

    def start(self) -> None:
        if self._thread is not None:
            return
        self._stop_event.clear()
        self._wake_r, self._wake_w = os.pipe2(os.O_NONBLOCK | os.O_CLOEXEC)
        self._thread = threading.Thread(
            target=self._wayland_loop,
            name="keyswitch-wayland-xkb",
            daemon=True,
        )
        self._thread.start()
        self._keymap_ready.wait(timeout=WAYLAND_KEYMAP_WAIT_SECONDS)

    def ensure_fallback_keymap(self, layouts: Sequence[str]) -> None:
        with self._lock:
            if self._keymap is not None and len(self._layout_names) >= len(layouts):
                return
        codes: list[str] = []
        variants: list[str] = []
        for entry in layouts:
            if "+" in entry:
                code, variant = entry.split("+", 1)
                codes.append(code)
                variants.append(variant)
            elif entry.isascii() and entry.islower() and len(entry) <= BITS_PER_BYTE and " " not in entry:
                # An XKB layout code (`us`, `ru`); sway names layouts by description (`Russian`).
                codes.append(entry)
                variants.append("")
        if not codes:
            codes = ["us", "ru"]
            variants = ["", ""]
        layout_bytes = ",".join(codes).encode("utf-8")
        variant_bytes = ",".join(variants).encode("utf-8")
        names = _XkbRuleNames(
            rules=b"evdev",
            model=b"pc105",
            layout=layout_bytes,
            variant=variant_bytes,
            options=b"",
        )
        km = self._xkb.xkb_keymap_new_from_names(self._xkb_ctx, ctypes.byref(names), 0)
        if km:
            self._install_keymap(int(km))

    def _install_keymap(self, km: int) -> None:
        st = self._xkb.xkb_state_new(km)
        if not st:
            self._xkb.xkb_keymap_unref(km)
            return
        count = int(self._xkb.xkb_keymap_num_layouts(km))
        names: list[str] = []
        for idx in range(count):
            raw = self._xkb.xkb_keymap_layout_get_name(km, idx)
            names.append(raw.decode("utf-8", errors="replace") if raw else f"layout-{idx}")
        shift_idx = int(self._xkb.xkb_keymap_mod_get_index(km, b"Shift"))
        lock_idx = int(self._xkb.xkb_keymap_mod_get_index(km, b"Lock"))
        invalid_mod = UINT32_MODULO_MASK
        with self._lock:
            old_state, old_km = self._state, self._keymap
            self._keymap = km
            self._state = int(st)
            self._layout_names = tuple(names)
            self._shift_mod = -1 if shift_idx == invalid_mod else shift_idx
            self._lock_mod = -1 if lock_idx == invalid_mod else lock_idx
            self._translations = {}
            self._symbols = {}
        if old_state is not None:
            self._xkb.xkb_state_unref(old_state)
        if old_km is not None:
            self._xkb.xkb_keymap_unref(old_km)
        self._keymap_ready.set()

    def translate_key(self, keycode: int, state: int, layout_index: int) -> str:
        key = (keycode, state & (SHIFT_MASK | LOCK_MASK), layout_index)
        cached = self._translations.get(key)
        if cached is not None:
            return cached
        with self._lock:
            if self._state is None:
                return ""
            depressed = (
                (1 << self._shift_mod)
                if (state & SHIFT_MASK) and self._shift_mod >= 0
                else 0
            )
            locked = (
                (1 << self._lock_mod)
                if (state & LOCK_MASK) and self._lock_mod >= 0
                else 0
            )
            self._xkb.xkb_state_update_mask(
                self._state,
                depressed,
                0,
                locked,
                0,
                0,
                max(0, layout_index),
            )
            codepoint = int(
                self._xkb.xkb_state_key_get_utf32(
                    self._state, keycode + WAYLAND_XKB_KEYCODE_OFFSET
                )
            )
            translations = self._translations
        char = chr(codepoint) if codepoint > 0 else ""
        result = char if char.isprintable() else ""
        translations[key] = result
        return result

    def keysym_name(self, keycode: int, layout_index: int = 0) -> str:
        key = (keycode, layout_index)
        cached = self._symbols.get(key)
        if cached is not None:
            return cached
        with self._lock:
            if self._state is None:
                return ""
            self._xkb.xkb_state_update_mask(
                self._state, 0, 0, 0, 0, 0, max(0, layout_index)
            )
            sym = int(
                self._xkb.xkb_state_key_get_one_sym(
                    self._state, keycode + WAYLAND_XKB_KEYCODE_OFFSET
                )
            )
            symbols = self._symbols
        name = ""
        if sym > 0:
            buf = ctypes.create_string_buffer(XKB_KEYSYM_NAME_BUFFER_BYTES)
            written = int(
                self._xkb.xkb_keysym_get_name(sym, buf, XKB_KEYSYM_NAME_BUFFER_BYTES)
            )
            if written > 0:
                name = buf.value.decode("ascii", errors="replace")
        symbols[key] = name
        return name

    def _wayland_loop(self) -> None:
        try:
            wl = ctypes.CDLL("libwayland-client.so.0")
        except OSError:
            self._keymap_ready.set()
            return

        wl.wl_display_connect.argtypes = [ctypes.c_char_p]
        wl.wl_display_connect.restype = ctypes.c_void_p
        wl.wl_display_disconnect.argtypes = [ctypes.c_void_p]
        wl.wl_display_disconnect.restype = None
        wl.wl_display_roundtrip.argtypes = [ctypes.c_void_p]
        wl.wl_display_roundtrip.restype = ctypes.c_int
        wl.wl_display_get_fd.argtypes = [ctypes.c_void_p]
        wl.wl_display_get_fd.restype = ctypes.c_int
        wl.wl_display_flush.argtypes = [ctypes.c_void_p]
        wl.wl_display_flush.restype = ctypes.c_int
        wl.wl_display_dispatch.argtypes = [ctypes.c_void_p]
        wl.wl_display_dispatch.restype = ctypes.c_int
        wl.wl_display_dispatch_pending.argtypes = [ctypes.c_void_p]
        wl.wl_display_dispatch_pending.restype = ctypes.c_int
        wl.wl_proxy_get_version.argtypes = [ctypes.c_void_p]
        wl.wl_proxy_get_version.restype = ctypes.c_uint32
        wl.wl_proxy_add_listener.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
        ]
        wl.wl_proxy_add_listener.restype = ctypes.c_int

        marshal_addr = ctypes.cast(wl.wl_proxy_marshal_flags, ctypes.c_void_p).value
        assert marshal_addr is not None
        marshal_new_obj = _MarshalNewObjFn(marshal_addr)
        marshal_bind = _MarshalBindFn(marshal_addr)

        wl_registry_iface = _WlInterface.in_dll(wl, "wl_registry_interface")
        wl_seat_iface = _WlInterface.in_dll(wl, "wl_seat_interface")
        wl_keyboard_iface = _WlInterface.in_dll(wl, "wl_keyboard_interface")

        dpy = wl.wl_display_connect(None)
        if not dpy:
            self._keymap_ready.set()
            return

        seat_holder: list[int] = []
        kbd_holder: list[int] = []

        def _on_keymap(
            _data: int | None, _kbd: int | None, fmt: int, fd: int, size: int
        ) -> None:
            raw_bytes = b""
            try:
                if fmt == WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1 and size > 0:
                    with mmap.mmap(fd, size, mmap.MAP_PRIVATE, mmap.PROT_READ) as mm:
                        raw_bytes = mm.read(size).rstrip(b"\x00")
            finally:
                try:
                    os.close(fd)
                except OSError:
                    pass
            if raw_bytes:
                km = self._xkb.xkb_keymap_new_from_string(
                    self._xkb_ctx, raw_bytes, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, 0
                )
                if km:
                    self._install_keymap(int(km))

        def _on_modifiers(
            _data: int | None,
            _kbd: int | None,
            _serial: int,
            _depressed: int,
            _latched: int,
            _locked: int,
            group: int,
        ) -> None:
            self._active_group = int(group)

        def _on_repeat_info(
            _data: int | None, _kbd: int | None, _rate: int, delay: int
        ) -> None:
            if delay > 0:
                self._repeat_delay_ms = int(delay)
                if self._on_repeat_delay is not None:
                    self._on_repeat_delay(int(delay))

        kbd_listener = _WlKeyboardListener(
            keymap=_KbdKeymapFn(_on_keymap),
            enter=_KbdEnterFn(lambda *_a: None),
            leave=_KbdLeaveFn(lambda *_a: None),
            key=_KbdKeyFn(lambda *_a: None),
            modifiers=_KbdModifiersFn(_on_modifiers),
            repeat_info=_KbdRepeatInfoFn(_on_repeat_info),
        )

        def _on_seat_caps(_data: int | None, seat: int | None, caps: int) -> None:
            if (caps & WL_SEAT_CAPABILITY_KEYBOARD) and not kbd_holder and seat:
                kbd = marshal_new_obj(
                    seat,
                    1,
                    ctypes.byref(wl_keyboard_iface),
                    wl.wl_proxy_get_version(seat),
                    0,
                    None,
                )
                if kbd:
                    kbd_holder.append(int(kbd))
                    wl.wl_proxy_add_listener(kbd, ctypes.byref(kbd_listener), None)

        seat_listener = _WlSeatListener(
            capabilities=_SeatCapabilitiesFn(_on_seat_caps),
            name=_SeatNameFn(lambda *_a: None),
        )

        def _on_global(
            _data: int | None,
            registry: int | None,
            name: int,
            iface: bytes | None,
            version: int,
        ) -> None:
            if iface == b"wl_seat" and not seat_holder and registry:
                bound_ver = min(int(version), WAYLAND_SEAT_INTERFACE_MAX_VERSION)
                seat = marshal_bind(
                    registry,
                    0,
                    ctypes.byref(wl_seat_iface),
                    bound_ver,
                    0,
                    name,
                    wl_seat_iface.name,
                    bound_ver,
                    None,
                )
                if seat:
                    seat_holder.append(int(seat))
                    wl.wl_proxy_add_listener(seat, ctypes.byref(seat_listener), None)

        reg_listener = _WlRegistryListener(
            global_cb=_RegGlobalFn(_on_global),
            global_remove_cb=_RegRemoveFn(lambda *_a: None),
        )

        try:
            reg = marshal_new_obj(
                dpy,
                1,
                ctypes.byref(wl_registry_iface),
                wl.wl_proxy_get_version(dpy),
                0,
                None,
            )
            if reg:
                wl.wl_proxy_add_listener(reg, ctypes.byref(reg_listener), None)
                # The registry names the seat, the seat its keyboard, the keyboard its keymap: one
                # round trip each, so a fallback keymap is never built while the real one is coming.
                for _ in range(WAYLAND_KEYMAP_ROUNDTRIPS):
                    wl.wl_display_roundtrip(dpy)
                    if self._keymap is not None:
                        break
            self._keymap_ready.set()

            dpy_fd = int(wl.wl_display_get_fd(dpy))
            wake_r = self._wake_r
            poller = select.poll()
            poller.register(dpy_fd, select.POLLIN | select.POLLERR | select.POLLHUP)
            if wake_r is not None:
                poller.register(wake_r, select.POLLIN)

            while not self._stop_event.is_set():
                wl.wl_display_dispatch_pending(dpy)
                wl.wl_display_flush(dpy)
                ready = dict(poller.poll())
                if self._stop_event.is_set() or (wake_r is not None and wake_r in ready):
                    break
                if dpy_fd in ready:
                    if ready[dpy_fd] & (select.POLLERR | select.POLLHUP):
                        break
                    if wl.wl_display_dispatch(dpy) < 0:
                        break
        finally:
            self._keymap_ready.set()
            wl.wl_display_disconnect(dpy)

    def close(self) -> None:
        self._stop_event.set()
        if self._wake_w is not None:
            try:
                os.write(self._wake_w, b"\x01")
            except OSError:
                pass
        thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=WAYLAND_IPC_TIMEOUT_SECONDS)
        self._thread = None
        for fd_attr in ("_wake_r", "_wake_w"):
            fd = getattr(self, fd_attr)
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass
                setattr(self, fd_attr, None)
        with self._lock:
            if self._state is not None:
                self._xkb.xkb_state_unref(self._state)
                self._state = None
            if self._keymap is not None:
                self._xkb.xkb_keymap_unref(self._keymap)
                self._keymap = None
            if self._xkb_ctx is not None:
                self._xkb.xkb_context_unref(self._xkb_ctx)
                self._xkb_ctx = None


# --- Dedicated GLib MainContext thread for D-Bus signals and KWin callbacks ---


class _DBusSignalThread:
    """Runs a private ``GLib.MainContext`` so D-Bus signals fire with or without GTK."""

    def __init__(self) -> None:
        self.context = GLib.MainContext.new()
        self._loop = GLib.MainLoop.new(self.context, False)
        self._started = threading.Event()
        self._thread = threading.Thread(
            target=self._run,
            name="keyswitch-wayland-dbus",
            daemon=True,
        )
        self._thread.start()
        self._started.wait(timeout=WAYLAND_IPC_TIMEOUT_SECONDS)

    def _run(self) -> None:
        self.context.push_thread_default()
        self._started.set()
        try:
            self._loop.run()
        finally:
            self.context.pop_thread_default()

    def invoke_sync(self, fn: Callable[[], None]) -> None:
        done = threading.Event()

        def _cb() -> bool:
            try:
                fn()
            finally:
                done.set()
            return GLib.SOURCE_REMOVE

        source = GLib.idle_source_new()
        source.set_callback(_cb)
        source.attach(self.context)
        done.wait(timeout=WAYLAND_IPC_TIMEOUT_SECONDS)

    def close(self) -> None:
        self._loop.quit()
        self.context.wakeup()
        if self._thread is not threading.current_thread():
            self._thread.join(timeout=WAYLAND_IPC_TIMEOUT_SECONDS)


# --- Desktop compositor adapters ---


class _DesktopAdapter(Protocol):
    label: str

    def setup_error(self) -> str: ...

    def layout_names(self) -> tuple[str, ...]: ...

    def current_layout_index(self) -> int: ...

    def select_layout_index(self, index: int) -> bool: ...

    def active_window(self) -> DesktopWindowInfo | None: ...

    def window_by_id(self, window_id: int) -> DesktopWindowInfo | None: ...

    def input_anchor(self) -> ScreenAnchor | None: ...

    def position_window(self, window: int, x: int, y: int) -> bool: ...

    def activate_window(self, window: int) -> bool: ...

    def isolated_layout(self) -> bool: ...

    def locked(self) -> bool: ...

    def close(self) -> None: ...


class GnomeShellAdapter:
    """GNOME Shell 45-51 Wayland integration via ``keyswitch@io.github.olegius88.KeySwitch``."""

    label = "GNOME Wayland"

    def __init__(
        self,
        bus: Gio.DBusConnection,
        loop_thread: _DBusSignalThread,
        on_lock_changed: Callable[[bool], None],
    ) -> None:
        self._bus = bus
        self._loop_thread = loop_thread
        self._on_lock_changed = on_lock_changed
        self._lock = threading.Lock()
        self._snapshot = DesktopSnapshot((), -1)
        self._windows: dict[int, DesktopWindowInfo] = {}
        self._sub_id = 0
        self._error = ""
        self._connect_extension()

    def _connect_extension(self) -> None:
        def _subscribe() -> None:
            self._sub_id = self._bus.signal_subscribe(
                GNOME_DBUS_NAME,
                GNOME_DBUS_INTERFACE,
                "StateChanged",
                GNOME_DBUS_PATH,
                None,
                Gio.DBusSignalFlags.NONE,
                self._on_state_signal,
            )

        self._loop_thread.invoke_sync(_subscribe)
        if self._refresh_state():
            return

        try:
            reply = self._bus.call_sync(
                GNOME_EXTENSIONS_BUS_NAME,
                GNOME_EXTENSIONS_PATH,
                GNOME_EXTENSIONS_INTERFACE,
                "GetExtensionInfo",
                GLib.Variant("(s)", (GNOME_EXTENSION_UUID,)),
                GLib.VariantType("(a{sv})"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            info = reply.unpack()[0]
        except GLib.Error as error:
            self._error = f"Не удалось обратиться к GNOME Shell: {error.message}"
            return

        if not info:
            self._error = (
                "Расширение GNOME Shell для KeySwitch ещё не загружено текущим сеансом: "
                "выйдите из учётной записи и войдите снова"
            )
            return

        try:
            self._bus.call_sync(
                GNOME_EXTENSIONS_BUS_NAME,
                GNOME_EXTENSIONS_PATH,
                GNOME_EXTENSIONS_INTERFACE,
                "EnableExtension",
                GLib.Variant("(s)", (GNOME_EXTENSION_UUID,)),
                GLib.VariantType("(b)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
        except GLib.Error as error:
            self._error = f"GNOME Shell не включил расширение KeySwitch: {error.message}"
            return

        deadline = time.monotonic() + WAYLAND_IPC_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            if self._refresh_state():
                self._error = ""
                return
            time.sleep(LAYOUT_SWITCH_POLL_SECONDS)
        self._error = (
            "Расширение GNOME Shell для KeySwitch не ответило после включения: "
            "проверьте расширения GNOME или перезайдите в сеанс"
        )

    def _on_state_signal(
        self,
        _conn: Gio.DBusConnection,
        _sender: str | None,
        _path: str,
        _iface: str,
        _signal: str,
        params: GLib.Variant,
    ) -> None:
        (raw_json,) = params.unpack()
        self._apply_raw_state(str(raw_json))

    def _refresh_state(self) -> bool:
        try:
            reply = self._bus.call_sync(
                GNOME_DBUS_NAME,
                GNOME_DBUS_PATH,
                GNOME_DBUS_INTERFACE,
                "GetState",
                None,
                GLib.VariantType("(s)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
        except GLib.Error:
            return False
        (raw_json,) = reply.unpack()
        self._apply_raw_state(str(raw_json))
        return True

    def _apply_raw_state(self, raw_json: str) -> None:
        try:
            snap = parse_gnome_state(raw_json)
        except ValueError:
            return
        with self._lock:
            prev_locked = self._snapshot.locked
            self._snapshot = snap
            if snap.window is not None:
                self._windows[snap.window.window_id] = snap.window
        if snap.locked != prev_locked:
            self._on_lock_changed(snap.locked)

    def setup_error(self) -> str:
        return self._error

    def layout_names(self) -> tuple[str, ...]:
        with self._lock:
            layouts = self._snapshot.layouts
        if not layouts:
            self._refresh_state()
            with self._lock:
                layouts = self._snapshot.layouts
        return layouts

    def current_layout_index(self) -> int:
        with self._lock:
            return self._snapshot.current_layout

    def select_layout_index(self, index: int) -> bool:
        try:
            reply = self._bus.call_sync(
                GNOME_DBUS_NAME,
                GNOME_DBUS_PATH,
                GNOME_DBUS_INTERFACE,
                "ActivateLayout",
                GLib.Variant("(u)", (index,)),
                GLib.VariantType("(b)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            ok = bool(reply.unpack()[0])
        except GLib.Error:
            return False
        if ok:
            self._refresh_state()
        return ok

    def active_window(self) -> DesktopWindowInfo | None:
        with self._lock:
            return self._snapshot.window

    def window_by_id(self, window_id: int) -> DesktopWindowInfo | None:
        with self._lock:
            return self._windows.get(window_id)

    def input_anchor(self) -> ScreenAnchor | None:
        self._refresh_state()
        with self._lock:
            win = self._snapshot.window
            ptr = self._snapshot.pointer
        win_id = win.window_id if win is not None else None
        if ptr is not None:
            return ScreenAnchor(ptr[0], ptr[1], win_id)
        if win is not None:
            center = win.center()
            if center is not None:
                return ScreenAnchor(center[0], center[1], win_id)
        return None

    def position_window(self, _window: int, x: int, y: int) -> bool:
        try:
            reply = self._bus.call_sync(
                GNOME_DBUS_NAME,
                GNOME_DBUS_PATH,
                GNOME_DBUS_INTERFACE,
                "MoveWindow",
                GLib.Variant("(usii)", (os.getpid(), LEARNING_PROMPT_WINDOW_TITLE, x, y)),
                GLib.VariantType("(b)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            return bool(reply.unpack()[0])
        except GLib.Error:
            return False

    def activate_window(self, window: int) -> bool:
        try:
            reply = self._bus.call_sync(
                GNOME_DBUS_NAME,
                GNOME_DBUS_PATH,
                GNOME_DBUS_INTERFACE,
                "ActivateWindow",
                GLib.Variant("(u)", (window,)),
                GLib.VariantType("(b)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            return bool(reply.unpack()[0])
        except GLib.Error:
            return False

    def isolated_layout(self) -> bool:
        with self._lock:
            return self._snapshot.isolated_layout

    def locked(self) -> bool:
        with self._lock:
            return self._snapshot.locked

    def close(self) -> None:
        sub_id = self._sub_id
        self._sub_id = 0
        if sub_id:
            self._loop_thread.invoke_sync(lambda: self._bus.signal_unsubscribe(sub_id))


class KdePlasmaAdapter:
    """KDE Plasma 6 / KWin Wayland integration via ``org.kde.keyboard`` and KWin Scripting."""

    label = "KDE Plasma Wayland"

    def __init__(
        self,
        bus: Gio.DBusConnection,
        loop_thread: _DBusSignalThread,
    ) -> None:
        self._bus = bus
        self._loop_thread = loop_thread
        self._lock = threading.Lock()
        self._id_map = KdeWindowIdMap()
        self._layouts: tuple[str, ...] = ()
        self._current_layout = 0
        self._window: DesktopWindowInfo | None = None
        self._windows: dict[int, DesktopWindowInfo] = {}
        self._pointer: tuple[int, int] | None = None
        self._layout_sub_id = 0
        self._list_sub_id = 0
        self._reg_id = 0
        self._watcher_js_path: Path | None = None
        self._error = ""
        self._setup()

    def _setup(self) -> None:
        node_info = Gio.DBusNodeInfo.new_for_xml(KWIN_CALLBACK_XML)
        iface_info = node_info.interfaces[0]

        def _register() -> None:
            self._reg_id = self._bus.register_object(
                KWIN_CALLBACK_PATH,
                iface_info,
                self._on_kwin_method_call,
                None,
                None,
            )
            self._layout_sub_id = self._bus.signal_subscribe(
                KDE_KEYBOARD_BUS_NAME,
                KDE_KEYBOARD_INTERFACE,
                "layoutChanged",
                KDE_KEYBOARD_PATH,
                None,
                Gio.DBusSignalFlags.NONE,
                self._on_layout_signal,
            )
            self._list_sub_id = self._bus.signal_subscribe(
                KDE_KEYBOARD_BUS_NAME,
                KDE_KEYBOARD_INTERFACE,
                "layoutListChanged",
                KDE_KEYBOARD_PATH,
                None,
                Gio.DBusSignalFlags.NONE,
                self._on_layout_list_signal,
            )

        self._loop_thread.invoke_sync(_register)
        self._refresh_layouts()
        self._refresh_current_layout()
        self._install_kwin_watcher()

    def _on_kwin_method_call(
        self,
        _conn: Gio.DBusConnection,
        _sender: str,
        _path: str,
        _iface: str,
        method: str,
        params: GLib.Variant,
        invocation: Gio.DBusMethodInvocation,
    ) -> None:
        if method == "Report":
            (raw_json,) = params.unpack()
            try:
                win, ptr = parse_kwin_report(str(raw_json), self._id_map)
            except ValueError:
                pass
            else:
                with self._lock:
                    self._window = win
                    if ptr is not None:
                        self._pointer = ptr
                    if win is not None:
                        self._windows[win.window_id] = win
            invocation.return_value(None)
            return
        invocation.return_dbus_error(
            "org.freedesktop.DBus.Error.UnknownMethod", "Unknown method"
        )

    def _on_layout_signal(
        self,
        _conn: Gio.DBusConnection,
        _sender: str | None,
        _path: str,
        _iface: str,
        _signal: str,
        params: GLib.Variant,
    ) -> None:
        (index,) = params.unpack()
        with self._lock:
            self._current_layout = int(index)

    def _on_layout_list_signal(
        self,
        _conn: Gio.DBusConnection,
        _sender: str | None,
        _path: str,
        _iface: str,
        _signal: str,
        _params: GLib.Variant,
    ) -> None:
        self._refresh_layouts()
        self._refresh_current_layout()

    def _refresh_layouts(self) -> None:
        try:
            reply = self._bus.call_sync(
                KDE_KEYBOARD_BUS_NAME,
                KDE_KEYBOARD_PATH,
                KDE_KEYBOARD_INTERFACE,
                "getLayoutsList",
                None,
                GLib.VariantType("(a(sss))"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            raw_list = reply.unpack()[0]
            parsed = parse_kde_layouts(
                [(str(a), str(b), str(c)) for a, b, c in raw_list]
            )
            with self._lock:
                self._layouts = parsed
        except GLib.Error as error:
            self._error = f"Служба раскладок KDE недоступна: {error.message}"

    def _refresh_current_layout(self) -> int:
        try:
            reply = self._bus.call_sync(
                KDE_KEYBOARD_BUS_NAME,
                KDE_KEYBOARD_PATH,
                KDE_KEYBOARD_INTERFACE,
                "getLayout",
                None,
                GLib.VariantType("(u)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            idx = int(reply.unpack()[0])
            with self._lock:
                self._current_layout = idx
            return idx
        except GLib.Error:
            with self._lock:
                return self._current_layout

    def _install_kwin_watcher(self) -> None:
        unique_name = self._bus.get_unique_name()
        if not unique_name:
            return
        plugin = f"{KWIN_WATCHER_PLUGIN}_{os.getpid()}"
        script = build_kwin_watcher_script(
            unique_name, KWIN_CALLBACK_PATH, KWIN_CALLBACK_IFACE
        )
        try:
            fd, temp_name = tempfile.mkstemp(prefix="keyswitch-kwin-", suffix=".js")
            os.write(fd, script.encode("utf-8"))
            os.close(fd)
            self._watcher_js_path = Path(temp_name)
            self._unload_kwin_plugin(plugin)
            reply = self._bus.call_sync(
                KWIN_BUS_NAME,
                KWIN_SCRIPTING_PATH,
                KWIN_SCRIPTING_INTERFACE,
                "loadScript",
                GLib.Variant("(ss)", (temp_name, plugin)),
                GLib.VariantType("(i)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            script_id = int(reply.unpack()[0])
            if script_id >= 0:
                self._bus.call_sync(
                    KWIN_BUS_NAME,
                    f"{KWIN_SCRIPTING_PATH}/Script{script_id}",
                    KWIN_SCRIPT_INTERFACE,
                    "run",
                    None,
                    None,
                    Gio.DBusCallFlags.NONE,
                    WAYLAND_DBUS_CALL_TIMEOUT_MS,
                    None,
                )
        except (GLib.Error, OSError):
            pass

    def _run_kwin_oneshot(self, code: str) -> bool:
        plugin = f"{KWIN_ACTION_PLUGIN}_{os.getpid()}"
        temp_path: Path | None = None
        try:
            fd, temp_name = tempfile.mkstemp(prefix="keyswitch-kwin-act-", suffix=".js")
            os.write(fd, code.encode("utf-8"))
            os.close(fd)
            temp_path = Path(temp_name)
            self._unload_kwin_plugin(plugin)
            reply = self._bus.call_sync(
                KWIN_BUS_NAME,
                KWIN_SCRIPTING_PATH,
                KWIN_SCRIPTING_INTERFACE,
                "loadScript",
                GLib.Variant("(ss)", (temp_name, plugin)),
                GLib.VariantType("(i)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            script_id = int(reply.unpack()[0])
            if script_id < 0:
                return False
            self._bus.call_sync(
                KWIN_BUS_NAME,
                f"{KWIN_SCRIPTING_PATH}/Script{script_id}",
                KWIN_SCRIPT_INTERFACE,
                "run",
                None,
                None,
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            self._unload_kwin_plugin(plugin)
            return True
        except (GLib.Error, OSError):
            return False
        finally:
            if temp_path is not None:
                try:
                    temp_path.unlink()
                except OSError:
                    pass

    def _unload_kwin_plugin(self, plugin: str) -> None:
        try:
            self._bus.call_sync(
                KWIN_BUS_NAME,
                KWIN_SCRIPTING_PATH,
                KWIN_SCRIPTING_INTERFACE,
                "unloadScript",
                GLib.Variant("(s)", (plugin,)),
                GLib.VariantType("(b)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
        except GLib.Error:
            pass

    def setup_error(self) -> str:
        return self._error

    def layout_names(self) -> tuple[str, ...]:
        with self._lock:
            layouts = self._layouts
        if not layouts:
            self._refresh_layouts()
            with self._lock:
                layouts = self._layouts
        return layouts

    def current_layout_index(self) -> int:
        # Asked at every key: answered from the layoutChanged signal, not by a call to KWin.
        with self._lock:
            return self._current_layout

    def select_layout_index(self, index: int) -> bool:
        try:
            reply = self._bus.call_sync(
                KDE_KEYBOARD_BUS_NAME,
                KDE_KEYBOARD_PATH,
                KDE_KEYBOARD_INTERFACE,
                "setLayout",
                GLib.Variant("(u)", (index,)),
                GLib.VariantType("(b)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            ok = bool(reply.unpack()[0])
        except GLib.Error:
            return False
        if ok:
            with self._lock:
                self._current_layout = index
        return ok

    def active_window(self) -> DesktopWindowInfo | None:
        with self._lock:
            return self._window

    def window_by_id(self, window_id: int) -> DesktopWindowInfo | None:
        with self._lock:
            return self._windows.get(window_id)

    def input_anchor(self) -> ScreenAnchor | None:
        unique_name = self._bus.get_unique_name()
        if unique_name:
            self._run_kwin_oneshot(
                build_kwin_action_script(
                    report_target=(unique_name, KWIN_CALLBACK_PATH, KWIN_CALLBACK_IFACE)
                )
            )
        with self._lock:
            win = self._window
            ptr = self._pointer
        win_id = win.window_id if win is not None else None
        if ptr is not None:
            return ScreenAnchor(ptr[0], ptr[1], win_id)
        if win is not None:
            center = win.center()
            if center is not None:
                return ScreenAnchor(center[0], center[1], win_id)
        return None

    def position_window(self, _window: int, x: int, y: int) -> bool:
        return self._run_kwin_oneshot(
            build_kwin_action_script(
                move_pid=os.getpid(),
                move_title=LEARNING_PROMPT_WINDOW_TITLE,
                x=x,
                y=y,
            )
        )

    def activate_window(self, window: int) -> bool:
        uuid_text = self._id_map.resolve(window)
        if not uuid_text:
            return False
        return self._run_kwin_oneshot(build_kwin_action_script(activate_uuid=uuid_text))

    def isolated_layout(self) -> bool:
        return False

    def locked(self) -> bool:
        return False

    def close(self) -> None:
        self._unload_kwin_plugin(f"{KWIN_WATCHER_PLUGIN}_{os.getpid()}")
        if self._watcher_js_path is not None:
            try:
                self._watcher_js_path.unlink()
            except OSError:
                pass
            self._watcher_js_path = None

        def _unsub() -> None:
            if self._layout_sub_id:
                self._bus.signal_unsubscribe(self._layout_sub_id)
                self._layout_sub_id = 0
            if self._list_sub_id:
                self._bus.signal_unsubscribe(self._list_sub_id)
                self._list_sub_id = 0
            if self._reg_id:
                self._bus.unregister_object(self._reg_id)
                self._reg_id = 0

        self._loop_thread.invoke_sync(_unsub)


class SwayIpcAdapter:
    """sway / i3-IPC Wayland integration over ``SWAYSOCK``."""

    label = "sway"

    def __init__(self, socket_path: str) -> None:
        self._socket_path = socket_path
        self._lock = threading.Lock()
        self._cmd_lock = threading.Lock()
        self._cmd_sock: socket.socket | None = None
        self._sub_sock: socket.socket | None = None
        self._stop_event = threading.Event()
        self._layouts: tuple[str, ...] = ()
        self._current_layout = 0
        self._window: DesktopWindowInfo | None = None
        self._windows: dict[int, DesktopWindowInfo] = {}
        self._error = ""
        self._sub_thread: threading.Thread | None = None
        self._setup()

    def _open_socket(self) -> socket.socket:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(WAYLAND_IPC_TIMEOUT_SECONDS)
        sock.connect(self._socket_path)
        return sock

    def _recv_exact(self, sock: socket.socket, count: int) -> bytes:
        buf = bytearray()
        while len(buf) < count:
            chunk = sock.recv(min(SWAY_IPC_READ_CHUNK_BYTES, count - len(buf)))
            if not chunk:
                raise OSError("sway IPC socket closed")
            buf.extend(chunk)
        return bytes(buf)

    def _roundtrip(self, message_type: int, payload: str = "") -> object:
        with self._cmd_lock:
            if self._cmd_sock is None:
                self._cmd_sock = self._open_socket()
            try:
                self._cmd_sock.sendall(encode_sway_ipc(message_type, payload))
                header = self._recv_exact(self._cmd_sock, SWAY_IPC_HEADER_BYTES)
                length, _reply_type = decode_sway_ipc_header(header)
                body = self._recv_exact(self._cmd_sock, length)
            except OSError:
                try:
                    self._cmd_sock.close()
                except OSError:
                    pass
                self._cmd_sock = None
                raise
        return json.loads(body.decode("utf-8", errors="replace"))

    def _setup(self) -> None:
        try:
            self._refresh_inputs()
            self._refresh_tree()
            self._sub_sock = self._open_socket()
            self._sub_sock.sendall(
                encode_sway_ipc(SWAY_IPC_SUBSCRIBE, json.dumps(["window", "input"]))
            )
            header = self._recv_exact(self._sub_sock, SWAY_IPC_HEADER_BYTES)
            length, _ = decode_sway_ipc_header(header)
            self._recv_exact(self._sub_sock, length)
            self._sub_sock.settimeout(None)
            self._sub_thread = threading.Thread(
                target=self._event_loop,
                name="keyswitch-sway-ipc",
                daemon=True,
            )
            self._sub_thread.start()
        except Exception as error:
            self._error = f"Не удалось подключиться к sway IPC: {error}"

    def _event_loop(self) -> None:
        sock = self._sub_sock
        if sock is None:
            return
        while not self._stop_event.is_set():
            try:
                header = self._recv_exact(sock, SWAY_IPC_HEADER_BYTES)
                length, event_type = decode_sway_ipc_header(header)
                self._recv_exact(sock, length)
            except (OSError, ValueError):
                return
            if event_type == SWAY_IPC_EVENT_INPUT:
                try:
                    self._refresh_inputs()
                except Exception:
                    pass
            elif event_type == SWAY_IPC_EVENT_WINDOW:
                try:
                    self._refresh_tree()
                except Exception:
                    pass

    def _refresh_inputs(self) -> None:
        data = self._roundtrip(SWAY_IPC_GET_INPUTS)
        layouts, active = parse_sway_inputs(data)
        with self._lock:
            self._layouts = layouts
            self._current_layout = active

    def _refresh_tree(self) -> object:
        tree = self._roundtrip(SWAY_IPC_GET_TREE)
        win = parse_sway_focused_window(tree)
        with self._lock:
            self._window = win
            if win is not None:
                self._windows[win.window_id] = win
        return tree

    def setup_error(self) -> str:
        return self._error

    def layout_names(self) -> tuple[str, ...]:
        with self._lock:
            layouts = self._layouts
        if not layouts:
            try:
                self._refresh_inputs()
            except (OSError, ValueError):
                return ()
            with self._lock:
                layouts = self._layouts
        return layouts

    def current_layout_index(self) -> int:
        # Asked at every key: answered from the input events sway sends, not by a round trip.
        with self._lock:
            return self._current_layout

    def select_layout_index(self, index: int) -> bool:
        try:
            reply = self._roundtrip(
                SWAY_IPC_RUN_COMMAND, sway_switch_layout_command(index)
            )
        except Exception:
            return False
        if isinstance(reply, list) and all(
            isinstance(item, dict) and item.get("success") is True for item in reply
        ):
            self._refresh_inputs()
            return True
        return False

    def active_window(self) -> DesktopWindowInfo | None:
        # Kept by the window events sway sends; a tree read at every key costs a round trip.
        with self._lock:
            return self._window

    def window_by_id(self, window_id: int) -> DesktopWindowInfo | None:
        # Geometry is asked for rarely (the prompt, a caret) and goes stale: a window gets its title
        # bar after the event announcing it, and no event follows. It is read afresh here.
        try:
            self._refresh_tree()
        except Exception:
            pass
        with self._lock:
            return self._windows.get(window_id)

    def input_anchor(self) -> ScreenAnchor | None:
        try:
            self._refresh_tree()
        except Exception:
            pass
        win = self.active_window()
        if win is None:
            return None
        center = win.center()
        if center is None:
            return ScreenAnchor(win.x, win.y, win.window_id)
        return ScreenAnchor(center[0], center[1], win.window_id)

    def position_window(self, _window: int, x: int, y: int) -> bool:
        try:
            tree = self._refresh_tree()
            con_id = find_sway_window_by_pid(
                tree, os.getpid(), title=LEARNING_PROMPT_WINDOW_TITLE
            )
            if con_id is None:
                return False
            reply = self._roundtrip(
                SWAY_IPC_RUN_COMMAND, sway_popup_move_command(con_id, x, y)
            )
            return isinstance(reply, list) and any(
                isinstance(item, dict) and item.get("success") is True for item in reply
            )
        except Exception:
            return False

    def activate_window(self, window: int) -> bool:
        try:
            reply = self._roundtrip(SWAY_IPC_RUN_COMMAND, sway_focus_command(window))
            return isinstance(reply, list) and any(
                isinstance(item, dict) and item.get("success") is True for item in reply
            )
        except Exception:
            return False

    def isolated_layout(self) -> bool:
        return False

    def locked(self) -> bool:
        return False

    def close(self) -> None:
        self._stop_event.set()
        for attr in ("_sub_sock", "_cmd_sock"):
            sock = cast(socket.socket | None, getattr(self, attr))
            if sock is not None:
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                try:
                    sock.close()
                except OSError:
                    pass
                setattr(self, attr, None)
        if (
            self._sub_thread is not None
            and self._sub_thread is not threading.current_thread()
        ):
            self._sub_thread.join(timeout=WAYLAND_IPC_TIMEOUT_SECONDS)
            self._sub_thread = None


class GenericWaylandAdapter:
    """Fallback adapter reading layouts and active group from ``wl_keyboard``."""

    label = "Wayland"

    def __init__(self, xkb: WaylandXkbMonitor) -> None:
        self._xkb = xkb

    def setup_error(self) -> str:
        return (
            "Текущий композитор Wayland не предоставляет интерфейс управления раскладкой "
            "(поддерживаются GNOME Shell, KDE Plasma 6 и sway)"
        )

    def layout_names(self) -> tuple[str, ...]:
        return self._xkb.layout_names

    def current_layout_index(self) -> int:
        return self._xkb.active_group

    def select_layout_index(self, _index: int) -> bool:
        return False

    def active_window(self) -> DesktopWindowInfo | None:
        return None

    def window_by_id(self, _window_id: int) -> DesktopWindowInfo | None:
        return None

    def input_anchor(self) -> ScreenAnchor | None:
        return None

    def position_window(self, _window: int, _x: int, _y: int) -> bool:
        return False

    def activate_window(self, _window: int) -> bool:
        return False

    def isolated_layout(self) -> bool:
        return False

    def locked(self) -> bool:
        return False

    def close(self) -> None:
        return None


# --- logind LockedHint / VT Active monitor ---


class LogindLockMonitor:
    """Watches ``org.freedesktop.login1`` so ``EVIOCGRAB`` is released on lock screens and VT switches."""

    def __init__(
        self,
        loop_thread: _DBusSignalThread,
        on_change: Callable[[], None],
    ) -> None:
        self._loop_thread = loop_thread
        self._on_change = on_change
        self._sys_bus: Gio.DBusConnection | None = None
        self._session_path = ""
        self._sub_ids: list[int] = []
        self._locked_hint = False
        self._session_active = True
        self._sleeping = False
        self._setup()

    @property
    def grab_blocked(self) -> bool:
        return self._locked_hint or not self._session_active or self._sleeping

    def _setup(self) -> None:
        try:
            self._sys_bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        except GLib.Error:
            return
        bus = self._sys_bus
        try:
            reply = bus.call_sync(
                LOGIND_BUS_NAME,
                LOGIND_USER_SELF_PATH,
                DBUS_PROPERTIES_IFACE,
                "Get",
                GLib.Variant("(ss)", (LOGIND_USER_IFACE, "Display")),
                GLib.VariantType("(v)"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            display_tuple = reply.unpack()[0]
            if isinstance(display_tuple, tuple) and len(display_tuple) > 1:
                self._session_path = str(display_tuple[1])
        except GLib.Error:
            self._session_path = ""

        if self._session_path:
            self._read_session_props()

        def _subscribe() -> None:
            assert self._sys_bus is not None
            if self._session_path:
                self._sub_ids.append(
                    self._sys_bus.signal_subscribe(
                        LOGIND_BUS_NAME,
                        DBUS_PROPERTIES_IFACE,
                        "PropertiesChanged",
                        self._session_path,
                        None,
                        Gio.DBusSignalFlags.NONE,
                        self._on_session_props,
                    )
                )
            self._sub_ids.append(
                self._sys_bus.signal_subscribe(
                    LOGIND_BUS_NAME,
                    LOGIND_MANAGER_IFACE,
                    "PrepareForSleep",
                    LOGIND_MANAGER_PATH,
                    None,
                    Gio.DBusSignalFlags.NONE,
                    self._on_prepare_sleep,
                )
            )

        self._loop_thread.invoke_sync(_subscribe)

    def _read_session_props(self) -> None:
        if self._sys_bus is None or not self._session_path:
            return
        try:
            reply = self._sys_bus.call_sync(
                LOGIND_BUS_NAME,
                self._session_path,
                DBUS_PROPERTIES_IFACE,
                "GetAll",
                GLib.Variant("(s)", (LOGIND_SESSION_IFACE,)),
                GLib.VariantType("(a{sv})"),
                Gio.DBusCallFlags.NONE,
                WAYLAND_DBUS_CALL_TIMEOUT_MS,
                None,
            )
            props = reply.unpack()[0]
            if isinstance(props, dict):
                if "LockedHint" in props:
                    self._locked_hint = bool(props["LockedHint"])
                if "Active" in props:
                    self._session_active = bool(props["Active"])
        except GLib.Error:
            pass

    def _on_session_props(
        self,
        _conn: Gio.DBusConnection,
        _sender: str | None,
        _path: str,
        _iface: str,
        _signal: str,
        params: GLib.Variant,
    ) -> None:
        unpacked = params.unpack()
        changed = unpacked[1] if len(unpacked) > 1 and isinstance(unpacked[1], dict) else {}
        updated = False
        if "LockedHint" in changed:
            self._locked_hint = bool(changed["LockedHint"])
            updated = True
        if "Active" in changed:
            self._session_active = bool(changed["Active"])
            updated = True
        if updated:
            self._on_change()

    def _on_prepare_sleep(
        self,
        _conn: Gio.DBusConnection,
        _sender: str | None,
        _path: str,
        _iface: str,
        _signal: str,
        params: GLib.Variant,
    ) -> None:
        (starting,) = params.unpack()
        self._sleeping = bool(starting)
        self._on_change()

    def close(self) -> None:
        sys_bus = self._sys_bus
        ids = list(self._sub_ids)
        self._sub_ids.clear()
        if sys_bus is not None and ids:
            def _unsub() -> None:
                for sid in ids:
                    sys_bus.signal_unsubscribe(sid)

            self._loop_thread.invoke_sync(_unsub)


# --- NativeWaylandAPI ---


def _dbus_name_has_owner(bus: Gio.DBusConnection, name: str) -> bool:
    try:
        reply = bus.call_sync(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            GLib.Variant("(s)", (name,)),
            GLib.VariantType("(b)"),
            Gio.DBusCallFlags.NONE,
            WAYLAND_DBUS_CALL_TIMEOUT_MS,
            None,
        )
        return bool(reply.unpack()[0])
    except GLib.Error:
        return False


def _helper_command(child_fd: int) -> list[str]:
    if "__compiled__" in globals():
        try:
            binary = str(Path("/proc/self/exe").resolve(strict=True))
        except OSError:
            binary = str(Path(sys.argv[0]).resolve())
        return [binary, "--wayland-input-helper", str(child_fd)]
    exe_name = Path(sys.executable).name.casefold()
    if not exe_name.startswith("python"):
        return [sys.executable, "--wayland-input-helper", str(child_fd)]
    return [sys.executable, "-m", "keyswitch", "--wayland-input-helper", str(child_fd)]


def _helper_environ() -> dict[str, str]:
    env = dict(os.environ)
    if "__compiled__" not in globals():
        package_parent = str(Path(__file__).resolve().parent.parent)
        existing = env.get("PYTHONPATH", "")
        parts = [item for item in existing.split(os.pathsep) if item]
        if package_parent not in parts:
            env["PYTHONPATH"] = os.pathsep.join((package_parent, *parts))
    return env


class NativeWaylandAPI:
    """Production ``WaylandAPI`` combining ``HelperClient``, ``WaylandXkbMonitor`` and desktop IPC."""

    def __init__(self) -> None:
        self._helper_client: HelperClient | None = None
        self._helper_proc: subprocess.Popen[bytes] | None = None
        self._stop_listener = threading.Event()
        self._caps_lock = False
        self._closed = True
        self._open()

    def _open(self) -> None:
        if not self._closed:
            return
        self._dbus_thread = _DBusSignalThread()
        self._session_bus: Gio.DBusConnection | None = None
        try:
            self._session_bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        except GLib.Error:
            self._session_bus = None

        self._xkb = WaylandXkbMonitor(on_repeat_delay=self._on_repeat_delay_changed)
        self._xkb.start()
        self._adapter = self._create_adapter()
        self._xkb.ensure_fallback_keymap(self._adapter.layout_names())
        self._logind = LogindLockMonitor(self._dbus_thread, self._sync_grab_state)
        self._closed = False

    def _create_adapter(self) -> _DesktopAdapter:
        sway_sock = os.environ.get("SWAYSOCK", "").strip()
        if sway_sock and not Path(sway_sock).is_socket():
            sway_sock = ""
        has_gnome = (
            self._session_bus is not None
            and _dbus_name_has_owner(self._session_bus, GNOME_EXTENSIONS_BUS_NAME)
        )
        has_kde = (
            self._session_bus is not None
            and _dbus_name_has_owner(self._session_bus, KDE_KEYBOARD_BUS_NAME)
        )
        kind = detect_desktop_kind(
            os.environ,
            has_gnome=has_gnome,
            has_kde=has_kde,
            sway_socket=sway_sock,
        )
        if kind == "sway" and sway_sock:
            return SwayIpcAdapter(sway_sock)
        if kind == "gnome" and self._session_bus is not None:
            return GnomeShellAdapter(
                self._session_bus,
                self._dbus_thread,
                lambda _locked: self._sync_grab_state(),
            )
        if kind == "kde" and self._session_bus is not None:
            return KdePlasmaAdapter(self._session_bus, self._dbus_thread)
        return GenericWaylandAdapter(self._xkb)

    def _on_repeat_delay_changed(self, delay_ms: int) -> None:
        client = self._helper_client
        if client is not None:
            try:
                client.set_grab(self._should_grab(), repeat_delay_ms=delay_ms)
            except Exception:
                pass

    def _should_grab(self) -> bool:
        return not (self._logind.grab_blocked or self._adapter.locked())

    def _sync_grab_state(self) -> None:
        client = self._helper_client
        if client is not None:
            try:
                client.set_grab(
                    self._should_grab(),
                    repeat_delay_ms=self._xkb.repeat_delay_ms,
                )
            except Exception:
                pass

    def desktop_name(self) -> str:
        wl_disp = os.environ.get("WAYLAND_DISPLAY", "").strip() or "wayland"
        return f"{self._adapter.label} ({wl_disp})"

    def permission_error(self) -> str:
        adapter_err = self._adapter.setup_error()
        if adapter_err:
            return adapter_err
        if not os.path.exists(UINPUT_PATH):
            return (
                "Устройство /dev/uinput отсутствует: загрузите модуль ядра uinput "
                "(sudo modprobe uinput)"
            )
        if not os.access(UINPUT_PATH, os.R_OK | os.W_OK):
            return (
                "Нет прав доступа к /dev/uinput: проверьте правила udev пакета keyswitch "
                "(60-keyswitch.rules) или перезайдите в сеанс"
            )
        os_probe = LinuxHelperOS()
        keyboards = [
            info
            for info in os_probe.list_event_devices()
            if classify_input_device(info) == "keyboard"
        ]
        if keyboards and not any(os.access(info.path, os.R_OK) for info in keyboards):
            return (
                "Нет прав чтения клавиатур /dev/input/event*: проверьте правила udev "
                "пакета keyswitch или перезайдите в сеанс"
            )
        return ""

    def layout_names(self) -> tuple[str, ...]:
        names = self._adapter.layout_names()
        if names:
            self._xkb.ensure_fallback_keymap(names)
            return names
        return self._xkb.layout_names

    def current_layout_index(self) -> int:
        return self._adapter.current_layout_index()

    def select_layout_index(self, index: int) -> bool:
        return self._adapter.select_layout_index(index)

    def translate_key(self, keycode: int, state: int, layout_index: int) -> str:
        return self._xkb.translate_key(keycode, state, layout_index)

    def keysym_name(self, keycode: int, layout_index: int = 0) -> str:
        return self._xkb.keysym_name(keycode, layout_index)

    def post_inputs(
        self, inputs: tuple[tuple[bool, int], ...], known_seq: int | None = None, action: bool = False
    ) -> int:
        client = self._helper_client
        if client is None:
            # Not 0: from the helper, 0 is a correction it refused (a key reached the window first).
            raise RuntimeError("Процесс ввода Wayland не запущен; текст не изменён")
        return client.emit_batch(inputs, known_seq, action=action)

    def active_application(self) -> str:
        win = self._adapter.active_window()
        return win.application if win is not None else ""

    def focused_window(self) -> int:
        win = self._adapter.active_window()
        return win.window_id if win is not None else 0

    def window_process_id(self, window: int) -> int:
        win = self._adapter.window_by_id(window)
        if win is None:
            active = self._adapter.active_window()
            if active is not None and active.window_id == window:
                win = active
        return win.pid if win is not None else 0

    def current_process_id(self) -> int:
        return os.getpid()

    def window_origin(self, window: int) -> tuple[int, int] | None:
        win = self._adapter.window_by_id(window)
        if win is None:
            active = self._adapter.active_window()
            if active is not None and (window == 0 or active.window_id == window):
                win = active
        if win is None:
            return None
        return (win.x, win.y)

    def window_is_password(self, window: int) -> bool:
        active = self._adapter.active_window()
        return active is not None and active.window_id == window and active.password

    def input_anchor(self) -> ScreenAnchor | None:
        return self._adapter.input_anchor()

    def position_window(self, window: int, x: int, y: int) -> bool:
        return self._adapter.position_window(window, x, y)

    def activate_window(self, window: int) -> bool:
        return self._adapter.activate_window(window)

    def isolated_layout(self) -> bool:
        return self._adapter.isolated_layout()

    def caps_lock_enabled(self) -> bool:
        return self._caps_lock

    def key_down(self, keycode: int) -> bool:
        client = self._helper_client
        if client is None:
            return False
        try:
            return client.query_key_down(keycode)
        except Exception:
            return False

    def monotonic_ms(self) -> int:
        return int(time.monotonic() * MILLISECONDS_PER_SECOND) & UINT32_MODULO_MASK

    def run_input_listener(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None],
    ) -> None:
        """Run the input helper until stopped; a helper that dies is started again.

        Its grab ends with it, so the keyboard keeps working, but KeySwitch would stay deaf: it is
        restarted up to WAYLAND_HELPER_RESTARTS times, and the parent forgets the keys it believed
        held down. A helper that fails at the first start is the caller's error.
        """

        self._open()
        self._stop_listener.clear()
        restarts = 0
        while True:
            exited = self._run_helper_once(listener, ready if restarts == 0 else None)
            if not exited or self._stop_listener.is_set():
                return
            if restarts >= WAYLAND_HELPER_RESTARTS:
                LOGGER.error("Процесс перехвата клавиатуры Wayland завершился %s раз подряд; перехват остановлен", restarts + 1)
                return
            restarts += 1
            LOGGER.warning("Процесс перехвата клавиатуры Wayland завершился; запуск заново (%s)", restarts)
            listener(NativeKeyEvent(True, 0, self.monotonic_ms(), input_reset=True))
            if self._stop_listener.wait(timeout=WAYLAND_HELPER_RESTART_DELAY_SECONDS):
                return

    def _run_helper_once(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None] | None,
    ) -> bool:
        """Run one helper process; True when it exited on its own (not stopped)."""

        parent_sock, child_sock = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
        os.set_inheritable(child_sock.fileno(), True)
        cmd = _helper_command(child_sock.fileno())
        proc: subprocess.Popen[bytes] | None = None
        client: HelperClient | None = None
        try:
            proc = subprocess.Popen(
                cmd,
                pass_fds=(child_sock.fileno(),),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                env=_helper_environ(),
            )
            child_sock.close()
            self._helper_proc = proc

            def _on_key(key: HelperKey) -> bool:
                return listener(NativeKeyEvent(
                    key.pressed, key.keycode, key.timestamp_ms, seq=key.seq,
                    delivered=not key.needs_verdict and not key.forced, forced=key.forced,
                ))

            def _on_pointer(timestamp_ms: int) -> None:
                listener(NativeKeyEvent(True, 0, timestamp_ms, pointer=True))

            def _on_repeat_reset(timestamp_ms: int) -> None:
                listener(NativeKeyEvent(True, 0, timestamp_ms, repeat_reset=True))

            def _on_input_reset() -> None:
                listener(NativeKeyEvent(True, 0, self.monotonic_ms(), input_reset=True))

            def _on_caps(enabled: bool) -> None:
                self._caps_lock = enabled

            client = HelperClient(
                parent_sock,
                on_key=_on_key,
                on_pointer=_on_pointer,
                on_repeat_reset=_on_repeat_reset,
                on_caps_lock=_on_caps,
                on_input_reset=_on_input_reset,
            )
            self._caps_lock = client.start()
            self._helper_client = client
            if not self._should_grab():
                client.set_grab(False, repeat_delay_ms=self._xkb.repeat_delay_ms)
            elif self._xkb.repeat_delay_ms != WAYLAND_REPEAT_DEFAULT_DELAY_MS:
                client.set_grab(True, repeat_delay_ms=self._xkb.repeat_delay_ms)
            if ready is not None:
                ready()
            while not self._stop_listener.wait(timeout=WAYLAND_IPC_TIMEOUT_SECONDS):
                if proc.poll() is not None or client.closed:
                    return True
            return False
        finally:
            self._helper_client = None
            try:
                child_sock.close()
            except OSError:
                pass
            if client is not None:
                client.close()
            else:
                try:
                    parent_sock.close()
                except OSError:
                    pass
            if proc is not None:
                self._helper_proc = None
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=WAYLAND_IPC_TIMEOUT_SECONDS)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait(timeout=WAYLAND_IPC_TIMEOUT_SECONDS)

    def stop_input_listener(self) -> None:
        self._stop_listener.set()

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self.stop_input_listener()
        self._logind.close()
        self._adapter.close()
        self._xkb.close()
        self._dbus_thread.close()
