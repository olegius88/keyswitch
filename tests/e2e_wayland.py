#!/usr/bin/env python3
"""Real Wayland desktop end-to-end suite (GNOME Shell, KDE Plasma 6, sway).

Exercises both ``--mode source`` (in-process ``WaylandBackend`` + ``KeySwitchEngine`` +
``LearningPromptWindow`` + ``RuleEditorWindow`` + AT-SPI field reader) and
``--mode packaged`` (installed Nuitka ``keyswitch-bin`` + ``--wayland-input-helper`` +
StatusNotifierItem tray + ``SIGSTOP``/``SIGCONT`` main-process hang safety) using
dedicated kernel ``/dev/uinput`` virtual physical keyboard and pointer devices.
"""

from __future__ import annotations

import argparse
from collections.abc import Callable
import errno
import fcntl
import grp
import json
import logging
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
from typing import cast

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, Gio, GLib, Gtk

from fixture_values.clock import (
    E2E_INTER_CASE_DELAY_MS,
    E2E_VERIFY_GRACE_MS,
    E2E_VERIFY_SETTLE_DELAY_MS,
    GDBUS_CALL_TIMEOUT_SECONDS,
    NATIVE_PACKAGE_E2E_FORCED_SHUTDOWN_TIMEOUT_SECONDS,
    NATIVE_PACKAGE_E2E_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS,
    NATIVE_PACKAGE_E2E_WARMUP_ATTEMPTS,
    SESSION_BUS_CALL_TIMEOUT_MS,
    X11_E2E_CONTEXT_RESOLUTION_VERIFY_DELAY_MS,
    X11_E2E_IDLE_PAUSE_VERIFY_DELAY_MS,
)
from fixture_values.ui import (
    E2E_TEST_ENTRY_MARGIN_PIXELS,
    E2E_TEST_WINDOW_HEIGHT_PIXELS,
    E2E_TEST_WINDOW_WIDTH_PIXELS,
)
from fixture_values.wayland import (
    WL_E2E_BUS_USB,
    WL_E2E_CAPS_DEADLINE_SECONDS,
    WL_E2E_DIAGNOSE_TIMEOUT_SECONDS,
    WL_E2E_FOCUS_DEADLINE_SECONDS,
    WL_E2E_HOTPLUG_PRODUCT_ID,
    WL_E2E_INTER_KEY_DELAY_SECONDS,
    WL_E2E_KBD_PRODUCT_ID,
    WL_E2E_KBD_VENDOR_ID,
    WL_E2E_KEY_P,
    WL_E2E_LOCK_SETTLE_SECONDS,
    WL_E2E_POLL_INTERVAL_SECONDS,
    WL_E2E_SIGSTOP_SETTLE_SECONDS,
    WL_E2E_SLOW_KEY_DELAY_MS,
    WL_E2E_TIMEOUT_SECONDS,
    WL_E2E_TRAY_READY_DEADLINE_SECONDS,
    WL_E2E_UDEV_GRAB_SETTLE_SECONDS,
    WL_E2E_ABS_MAX,
    WL_E2E_ABS_X,
    WL_E2E_ABS_Y,
    WL_E2E_TABLET_PRODUCT_ID,
    WL_E2E_UI_ABS_SETUP,
    WL_E2E_UI_SET_ABSBIT,
    WL_E2E_US_KEYMAP,
    WL_E2E_WATCHDOG_DEADLINE_SECONDS,
)
from keyswitch.config import SettingsStore
from keyswitch.constants.geometry import CENTERING_DIVISOR
from keyswitch.constants.units import MILLISECONDS_PER_SECOND
from keyswitch.constants.wayland import (
    EV_KEY,
    EV_LED,
    EV_ABS,
    EV_REP,
    EV_SYN,
    EVDEV_BTN_MOUSE,
    EVDEV_KEY_A,
    EVDEV_KEY_BACKSPACE,
    EVDEV_KEY_CAPSLOCK,
    EVDEV_KEY_ENTER,
    EVDEV_KEY_LEFTALT,
    EVDEV_KEY_LEFTCTRL,
    EVDEV_KEY_LEFTSHIFT,
    EVDEV_KEY_PAUSE,
    EVDEV_KEY_Z,
    EVDEV_READ_BATCH_EVENTS,
    EVIOCGNAME_256,
    EVIOCGRAB,
    EVDEV_NAME_BUFFER_BYTES,
    KEY_VALUE_PRESS,
    KEY_VALUE_RELEASE,
    LED_CAPSL,
    SYN_REPORT,
    UI_DEV_CREATE,
    UI_DEV_DESTROY,
    UI_DEV_SETUP,
    UI_SET_EVBIT,
    UI_SET_KEYBIT,
    UI_SET_LEDBIT,
    UINPUT_MAX_NAME_BYTES,
    WAYLAND_FORWARDED_LEDS,
    WAYLAND_UINPUT_MAX_KEYCODE,
    WAYLAND_UINPUT_VERSION,
)
from keyswitch.engine import KeySwitchEngine, LearningPrompt
from keyswitch.history import HistoryStore
from keyswitch.learning import InvalidRule, LearningStore
from keyswitch.learning_prompt import LearningPromptWindow, RuleEditorWindow
from keyswitch.rule_editor import RuleDraft, build_rule, draft_from_prompt
from keyswitch.tray import ITEM_INTERFACE, MENU_INTERFACE, MENU_PATH, OBJECT_PATH
from keyswitch.wayland_backend import WaylandBackend


# struct uinput_setup {struct input_id id; char name[UINPUT_MAX_NAME_SIZE]; __u32 ff_effects_max;}
_uinput_setup_struct = struct.Struct("@HHHH80sI")
# struct input_event {struct timeval time; __u16 type; __u16 code; __s32 value;}
_input_event_struct = struct.Struct("@llHHi")
# struct uinput_abs_setup {__u16 code; struct input_absinfo absinfo;}
_abs_setup_struct = struct.Struct("@H2xiiiiii")

WATCHER_BUS_NAME = "org.kde.StatusNotifierWatcher"
DOCUMENT_PORTAL_NAME = "org.freedesktop.portal.Documents"
WATCHER_INTERFACE = "org.kde.StatusNotifierWatcher"
WATCHER_PATH = "/StatusNotifierWatcher"


class TestWatcher(dbus.service.Object):
    """Minimal StatusNotifierWatcher if the compositor does not already own the name."""

    def __init__(self, bus: dbus.Bus) -> None:
        self.registered = False
        self.service = ""
        self._bus_name = dbus.service.BusName(WATCHER_BUS_NAME, bus)
        super().__init__(self._bus_name, WATCHER_PATH)

    @dbus.service.method(WATCHER_INTERFACE, in_signature="s", out_signature="")
    def RegisterStatusNotifierItem(self, service: str) -> None:
        self.service = service
        self.registered = True


def pump_gtk() -> None:
    ctx = GLib.MainContext.default()
    while ctx.pending():
        ctx.iteration(False)


def sleep_pumping(seconds: float) -> None:
    deadline = time.monotonic() + seconds
    while True:
        pump_gtk()
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        time.sleep(min(WL_E2E_POLL_INTERVAL_SECONDS, remaining))
    pump_gtk()


def wait_until(predicate: Callable[[], bool], timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        pump_gtk()
        if predicate():
            return True
        time.sleep(WL_E2E_POLL_INTERVAL_SECONDS)
    pump_gtk()
    return predicate()


class UinputPhysicalKeyboard:
    """Simulated hardware USB keyboard fed into the kernel input subsystem."""

    def __init__(self, name: str = "USB HID Test Keyboard", product_id: int = WL_E2E_KBD_PRODUCT_ID) -> None:
        self.name = name
        self._fd = os.open("/dev/uinput", os.O_RDWR | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            for ev in (EV_KEY, EV_SYN, EV_LED, EV_REP):
                fcntl.ioctl(self._fd, UI_SET_EVBIT, ev)
            for code in range(1, WAYLAND_UINPUT_MAX_KEYCODE + 1):
                fcntl.ioctl(self._fd, UI_SET_KEYBIT, code)
            for led in WAYLAND_FORWARDED_LEDS:
                fcntl.ioctl(self._fd, UI_SET_LEDBIT, led)
            setup = _uinput_setup_struct.pack(
                WL_E2E_BUS_USB,
                WL_E2E_KBD_VENDOR_ID,
                product_id,
                WAYLAND_UINPUT_VERSION,
                name.encode("utf-8")[: UINPUT_MAX_NAME_BYTES - 1].ljust(UINPUT_MAX_NAME_BYTES, b"\x00"),
                0,
            )
            fcntl.ioctl(self._fd, UI_DEV_SETUP, setup)
            fcntl.ioctl(self._fd, UI_DEV_CREATE)
        except Exception:
            os.close(self._fd)
            raise
        self._closed = False
        self._leds: dict[int, int] = {}

    def _emit(self, events: list[tuple[int, int, int]]) -> None:
        payload = b"".join(
            _input_event_struct.pack(0, 0, ev_type, code, value)
            for ev_type, code, value in events
        )
        os.write(self._fd, payload)

    def tap_code(self, keycode: int, *, shift: bool = False) -> None:
        if shift:
            self._emit([(EV_KEY, EVDEV_KEY_LEFTSHIFT, KEY_VALUE_PRESS), (EV_SYN, SYN_REPORT, 0)])
            time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)
        self._emit([(EV_KEY, keycode, KEY_VALUE_PRESS), (EV_SYN, SYN_REPORT, 0)])
        time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)
        self._emit([(EV_KEY, keycode, KEY_VALUE_RELEASE), (EV_SYN, SYN_REPORT, 0)])
        if shift:
            time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)
            self._emit([(EV_KEY, EVDEV_KEY_LEFTSHIFT, KEY_VALUE_RELEASE), (EV_SYN, SYN_REPORT, 0)])
        time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)

    def type_physical(self, text: str, *, delay_seconds: float = WL_E2E_INTER_KEY_DELAY_SECONDS) -> None:
        for ch in text:
            if ch == "\n":
                self.tap_code(EVDEV_KEY_ENTER)
            else:
                spec = WL_E2E_US_KEYMAP.get(ch)
                if spec is None:
                    raise RuntimeError(f"Unsupported physical character {ch!r}")
                code, shifted = spec
                self.tap_code(code, shift=shifted)
            if delay_seconds > WL_E2E_INTER_KEY_DELAY_SECONDS:
                sleep_pumping(delay_seconds - WL_E2E_INTER_KEY_DELAY_SECONDS)

    def chord(self, modifiers: tuple[int, ...], keycode: int) -> None:
        for mod in modifiers:
            self._emit([(EV_KEY, mod, KEY_VALUE_PRESS), (EV_SYN, SYN_REPORT, 0)])
            time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)
        self._emit([(EV_KEY, keycode, KEY_VALUE_PRESS), (EV_SYN, SYN_REPORT, 0)])
        time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)
        self._emit([(EV_KEY, keycode, KEY_VALUE_RELEASE), (EV_SYN, SYN_REPORT, 0)])
        for mod in reversed(modifiers):
            time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)
            self._emit([(EV_KEY, mod, KEY_VALUE_RELEASE), (EV_SYN, SYN_REPORT, 0)])
        time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)

    def clear_field(self) -> None:
        self.chord((EVDEV_KEY_LEFTCTRL,), EVDEV_KEY_A)
        self.tap_code(EVDEV_KEY_BACKSPACE)

    def ensure_caps_lock_off(self, target: "TargetApp") -> None:
        """Caps Lock outlives a run that failed with it on: the focused target reads the session's state."""

        if target.caps_lock:
            self.tap_code(EVDEV_KEY_CAPSLOCK)
            if not wait_until(lambda: not target.caps_lock, WL_E2E_CAPS_DEADLINE_SECONDS):
                raise RuntimeError("Caps Lock left on by an earlier run could not be turned off")

    def drain_leds(self) -> dict[int, int]:
        size = _input_event_struct.size
        while True:
            try:
                chunk = os.read(self._fd, size * EVDEV_READ_BATCH_EVENTS)
            except BlockingIOError:
                break
            if not chunk:
                break
            count = len(chunk) // size
            for idx in range(count):
                _sec, _usec, ev_type, code, value = _input_event_struct.unpack_from(chunk, idx * size)
                if ev_type == EV_LED:
                    self._leds[int(code)] = int(value)
        return dict(self._leds)

    def find_event_node(self) -> Path | None:
        for node in sorted(Path("/dev/input").glob("event*")):
            try:
                fd = os.open(str(node), os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
            except OSError:
                continue
            try:
                buf = bytearray(EVDEV_NAME_BUFFER_BYTES)
                fcntl.ioctl(fd, EVIOCGNAME_256, buf)
                dev_name = bytes(buf).split(b"\x00", 1)[0].decode("utf-8", errors="replace")
                if dev_name == self.name:
                    return node
            except OSError:
                pass
            finally:
                os.close(fd)
        return None

    def is_exclusively_grabbed(self) -> bool:
        node = self.find_event_node()
        if node is None:
            return False
        fd = os.open(str(node), os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            fcntl.ioctl(fd, EVIOCGRAB, 1)
            fcntl.ioctl(fd, EVIOCGRAB, 0)
            return False
        except OSError as error:
            return error.errno == errno.EBUSY
        finally:
            os.close(fd)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            fcntl.ioctl(self._fd, UI_DEV_DESTROY)
        except OSError:
            pass
        os.close(self._fd)


class UinputPhysicalPointer:
    """Simulated absolute USB pointer (a tablet, as QEMU gives one) that clicks where the test says.

    A relative mouse clicks wherever the pointer happens to be: in a desktop with a window under it
    the click takes the keyboard focus there, and the rest of the run types into that window.
    """

    def __init__(self, name: str = "USB Test Tablet") -> None:
        self._fd = os.open("/dev/uinput", os.O_RDWR | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            for ev in (EV_KEY, EV_ABS, EV_SYN):
                fcntl.ioctl(self._fd, UI_SET_EVBIT, ev)
            fcntl.ioctl(self._fd, UI_SET_KEYBIT, EVDEV_BTN_MOUSE)
            for axis in (WL_E2E_ABS_X, WL_E2E_ABS_Y):
                fcntl.ioctl(self._fd, WL_E2E_UI_SET_ABSBIT, axis)
                # struct uinput_abs_setup {__u16 code; struct input_absinfo {value, min, max, fuzz, flat, resolution}}
                fcntl.ioctl(self._fd, WL_E2E_UI_ABS_SETUP, _abs_setup_struct.pack(axis, 0, 0, WL_E2E_ABS_MAX, 0, 0, 0))
            setup = _uinput_setup_struct.pack(
                WL_E2E_BUS_USB,
                WL_E2E_KBD_VENDOR_ID,
                WL_E2E_TABLET_PRODUCT_ID,
                WAYLAND_UINPUT_VERSION,
                name.encode("utf-8")[: UINPUT_MAX_NAME_BYTES - 1].ljust(UINPUT_MAX_NAME_BYTES, b"\x00"),
                0,
            )
            fcntl.ioctl(self._fd, UI_DEV_SETUP, setup)
            fcntl.ioctl(self._fd, UI_DEV_CREATE)
        except Exception:
            os.close(self._fd)
            raise
        self._closed = False

    def _emit(self, events: list[tuple[int, int, int]]) -> None:
        os.write(self._fd, b"".join(_input_event_struct.pack(0, 0, *event) for event in events))

    def click_at(self, x: int, y: int, screen: tuple[int, int, int, int]) -> None:
        """Press and release the left button at the logical screen point (x, y) of ``screen``'s extents."""

        left, top, width, height = screen
        position = [
            (EV_ABS, WL_E2E_ABS_X, (x - left) * WL_E2E_ABS_MAX // max(1, width - 1)),
            (EV_ABS, WL_E2E_ABS_Y, (y - top) * WL_E2E_ABS_MAX // max(1, height - 1)),
            (EV_SYN, SYN_REPORT, 0),
        ]
        self._emit(position)
        time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)
        for value in (KEY_VALUE_PRESS, KEY_VALUE_RELEASE):
            self._emit([(EV_KEY, EVDEV_BTN_MOUSE, value), (EV_SYN, SYN_REPORT, 0)])
            time.sleep(WL_E2E_INTER_KEY_DELAY_SECONDS)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            fcntl.ioctl(self._fd, UI_DEV_DESTROY)
        except OSError:
            pass
        os.close(self._fd)


def screen_extents() -> tuple[int, int, int, int]:
    """The logical bounds of all monitors, which an absolute pointer spans."""

    display = Gdk.Display.get_default()
    assert display is not None
    monitors = display.get_monitors()
    rects = [cast(Gdk.Monitor, monitors.get_item(index)).get_geometry() for index in range(monitors.get_n_items())]
    left = min(rect.x for rect in rects)
    top = min(rect.y for rect in rects)
    right = max(rect.x + rect.width for rect in rects)
    bottom = max(rect.y + rect.height for rect in rects)
    return left, top, right - left, bottom - top


def widget_screen_center(backend: WaylandBackend, window_id: int, target: "TargetApp", widget: str) -> tuple[int, int]:
    """Where a target widget is on screen: the compositor's frame of its window plus its place in it."""

    origin = backend.window_origin(window_id)
    ok, x, y, width, height = target.bounds(widget)
    if origin is None or not ok:
        raise RuntimeError(f"Cannot place the click: window origin {origin!r}, widget bounds known {ok}")
    return origin[0] + int(x + width / CENTERING_DIVISOR), origin[1] + int(y + height / CENTERING_DIVISOR)


def verify_unprivileged_uaccess() -> None:
    if os.geteuid() == 0:
        raise RuntimeError("Wayland E2E must run as an unprivileged desktop user, not root")
    group_names = {grp.getgrgid(gid).gr_name for gid in os.getgroups()}
    if "input" in group_names:
        raise RuntimeError(
            f"User belongs to 'input' group ({sorted(group_names)}); uaccess ACLs would not be proven"
        )
    if not os.access("/dev/uinput", os.R_OK | os.W_OK):
        raise RuntimeError("Unprivileged user lacks R/W ACL on /dev/uinput: run sudo keyswitch --enable-wayland-input first")
    print("UNPRIVILEGED_UACCESS_OK")


def set_session_locked_hint(locked: bool) -> bool:
    session_id = os.environ.get("XDG_SESSION_ID", "").strip()
    if not session_id:
        return False
    session_path = f"/org/freedesktop/login1/session/{session_id}"
    completed = subprocess.run(
        [
            "sudo",
            "-n",
            "gdbus",
            "call",
            "--system",
            "--dest",
            "org.freedesktop.login1",
            "--object-path",
            session_path,
            "--method",
            "org.freedesktop.login1.Session.SetLockedHint",
            "true" if locked else "false",
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=GDBUS_CALL_TIMEOUT_SECONDS,
    )
    return completed.returncode == 0


class TargetApp:
    """The GTK4 window of another process the suite types into (tests/e2e_wayland_target.py).

    KeySwitch neither follows the focus into a window of its own process nor reads its fields, so a
    window in the suite's process would hide exactly what a user's application goes through.
    """

    def __init__(self) -> None:
        self._process = subprocess.Popen(
            [sys.executable, str(Path(__file__).with_name("e2e_wayland_target.py"))],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        self._lock = threading.Lock()
        self._texts = {"entry": "", "password": ""}
        self._bounds: dict[str, tuple[bool, float, float, float, float]] = {}
        self._done = 0
        self._seq = 0
        self.activations: list[str] = []
        self.clicks: list[tuple[float, float]] = []
        self.active = False
        self.caps_lock = False
        self._reader = threading.Thread(target=self._read, name="e2e-target-reader", daemon=True)
        self._reader.start()
        self.entry = RemoteEntry(self, "entry")
        self.password = RemoteEntry(self, "password")

    def _read(self) -> None:
        stdout = self._process.stdout
        assert stdout is not None
        for line in stdout:
            event = json.loads(line)
            with self._lock:
                match event["event"]:
                    case "text":
                        self._texts[event["widget"]] = event["text"]
                    case "activate":
                        self.activations.append(event["text"])
                    case "active":
                        self.active = bool(event["value"])
                    case "caps":
                        self.caps_lock = bool(event["value"])
                    case "bounds":
                        self._bounds[event["widget"]] = (
                            bool(event["ok"]), event["x"], event["y"], event["width"], event["height"],
                        )
                    case "done":
                        self._done = max(self._done, int(event["seq"]))
                    case "click":
                        self.clicks.append((float(event["x"]), float(event["y"])))
                    case _:
                        pass

    def command(self, cmd: str, **fields: object) -> None:
        stdin = self._process.stdin
        assert stdin is not None
        self._seq += 1
        seq = self._seq
        stdin.write(json.dumps({"cmd": cmd, "seq": seq, **fields}, ensure_ascii=False) + "\n")
        stdin.flush()
        if not wait_until(lambda: self._done >= seq, WL_E2E_FOCUS_DEADLINE_SECONDS):
            raise RuntimeError(f"The target application did not answer {cmd!r}")

    def text(self, widget: str) -> str:
        with self._lock:
            return self._texts[widget]

    def bounds(self, widget: str) -> tuple[bool, float, float, float, float]:
        self.command("bounds", widget=widget)
        with self._lock:
            return self._bounds[widget]

    def present(self) -> None:
        self.command("present")

    def destroy(self) -> None:
        if self._process.poll() is None:
            try:
                self.command("quit")
            except (OSError, RuntimeError):
                pass
            try:
                self._process.wait(timeout=NATIVE_PACKAGE_E2E_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS)
            except subprocess.TimeoutExpired:
                self._process.kill()


class RemoteEntry:
    """The few Gtk.Entry calls the suite makes, on an entry of the target application."""

    def __init__(self, target: TargetApp, widget: str) -> None:
        self._target = target
        self.widget = widget

    def get_text(self) -> str:
        return self._target.text(self.widget)

    def set_text(self, text: str) -> None:
        self._target.command("set_text", widget=self.widget, text=text)

    def grab_focus(self) -> None:
        self._target.command("focus", widget=self.widget)


def ensure_entry_focused(
    window: TargetApp,
    entry: RemoteEntry,
    kbd: UinputPhysicalKeyboard,
) -> None:
    deadline = time.monotonic() + WL_E2E_FOCUS_DEADLINE_SECONDS
    while time.monotonic() < deadline:
        window.present()
        entry.grab_focus()
        sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
        entry.set_text("")
        pump_gtk()
        kbd.tap_code(EVDEV_KEY_A)
        if wait_until(lambda: len(entry.get_text()) > 0, E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND):
            kbd.clear_field()
            wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
            return
    raise RuntimeError(f"GTK4 test entry never received Wayland keyboard focus (text={entry.get_text()!r})")


def session_bus_names() -> list[str]:
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    names: list[str] = bus.call_sync(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
        None,
        GLib.VariantType.new("(as)"),
        Gio.DBusCallFlags.NONE,
        SESSION_BUS_CALL_TIMEOUT_MS,
        None,
    ).unpack()[0]
    return names


def run_source_e2e(expected_desktop: str) -> int:
    verify_unprivileged_uaccess()
    Gtk.init()
    portal_before = DOCUMENT_PORTAL_NAME in session_bus_names()
    logging.basicConfig(level=logging.INFO, format="%(message)s")

    kbd = UinputPhysicalKeyboard()
    mouse = UinputPhysicalPointer()
    sleep_pumping(WL_E2E_UDEV_GRAB_SETTLE_SECONDS)

    temporary = tempfile.TemporaryDirectory()
    root = Path(temporary.name)
    settings = SettingsStore(root / "config.json")
    settings.set("general.keep_history", True)
    settings.set("detection.early_switch", False)
    settings.set("diagnostics.technical_logging", True)
    history = HistoryStore(root / "history.jsonl")

    backend = WaylandBackend()
    probe = backend.probe()
    print(f"probe={probe!r}")
    if not probe.available or probe.session_type != "wayland":
        raise RuntimeError(f"WaylandBackend probe unavailable: {probe!r}")
    if expected_desktop and expected_desktop.casefold() not in probe.display.casefold():
        raise RuntimeError(f"Expected desktop {expected_desktop!r}, got {probe.display!r}")

    engine = KeySwitchEngine(settings, history, backend)
    correction_modes: list[str] = []
    engine.subscribe_corrections(lambda plan: correction_modes.append(plan.mode))

    application = Gtk.Application(application_id="io.github.olegius88.KeySwitchWaylandE2E")
    application.register(None)

    window = TargetApp()
    entry = window.entry
    password_entry = window.password
    activated_snapshots = window.activations

    learning_prompt = LearningPromptWindow(
        application,
        backend,
        engine.confirm_learning_prompt,
        engine.dismiss_learning_prompt,
    )

    def apply_learning_prompt(prompt: LearningPrompt | None) -> bool:
        if prompt is None:
            learning_prompt.hide_prompt()
        else:
            learning_prompt.show_prompt(prompt)
        return GLib.SOURCE_REMOVE

    def queue_learning_prompt(prompt: LearningPrompt | None) -> None:
        GLib.idle_add(apply_learning_prompt, prompt)

    engine.subscribe_learning_prompts(queue_learning_prompt)
    rule_windows: list[RuleEditorWindow] = []
    offer_windows: list[int] = []

    def accept_rule(draft: RuleDraft) -> str:
        try:
            engine.add_learning_rule(build_rule(draft), application=draft.application)
        except InvalidRule as error:
            return str(error)
        return ""

    def rule_window_closed() -> None:
        for offer_window in offer_windows:
            backend.restore_window(offer_window)

    def open_rule_window(prompt: LearningPrompt) -> bool:
        editor = RuleEditorWindow(
            application,
            draft_from_prompt(prompt),
            accept_rule,
            rule_window_closed,
            engine.set_rule_editor_open,
        )
        rule_windows.append(editor)
        editor.present()
        return GLib.SOURCE_REMOVE

    def request_rule_window(prompt: LearningPrompt) -> None:
        GLib.idle_add(open_rule_window, prompt)

    engine.subscribe_rule_requests(request_rule_window)

    window.present()
    entry.grab_focus()
    sleep_pumping(WL_E2E_UDEV_GRAB_SETTLE_SECONDS)

    original_group = backend.current_group()
    settings.set("detection.respect_manual_layout", False)
    engine.start()
    try:
        if not wait_until(kbd.is_exclusively_grabbed, WL_E2E_FOCUS_DEADLINE_SECONDS):
            raise RuntimeError("WaylandInputHelper did not acquire EVIOCGRAB on virtual physical keyboard")
        print("EVIOCGRAB_ACQUIRED_OK")

        backend.switch_group(0)
        ensure_entry_focused(window, entry, kbd)
        kbd.ensure_caps_lock_off(window)
        history.clear()
        settings.set("detection.respect_manual_layout", True)

        focus = backend.focused_window()
        print(f"focused_window={focus!r} active_app={backend.active_application()!r}")
        if focus is None or focus.window <= 0 or focus.own:
            raise RuntimeError(f"Compositor adapter did not report the target's window: {focus!r}")

        # 1. Core X11-parity typing suite (15 cases).
        cases = (
            ("EN pause correction", 0, "ghbdtn", "привет", 1, X11_E2E_IDLE_PAUSE_VERIFY_DELAY_MS),
            ("RU keys to English", 1, "hello ", "hello ", 0, E2E_VERIFY_SETTLE_DELAY_MS),
            ("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1, E2E_VERIFY_SETTLE_DELAY_MS),
            ("return to EN before punctuation test", 1, "hello ", "hello ", 0, E2E_VERIFY_SETTLE_DELAY_MS),
            ("ambiguous punctuation keeps its glyph after idle", 0, "ghbdtn,", "привет,", 1, X11_E2E_IDLE_PAUSE_VERIFY_DELAY_MS),
            ("manual layout switch protects next word", 0, "ghbdtn ", "ghbdtn ", 0, E2E_VERIFY_SETTLE_DELAY_MS),
            ("manual protection is consumed once", 0, "ghbdtn ", "привет ", 1, E2E_VERIFY_SETTLE_DELAY_MS),
            ("short Russian word switches to English with its neighbour", 1, "if hello ", "if hello ", 0, X11_E2E_CONTEXT_RESOLUTION_VERIFY_DELAY_MS),
            ("manual Russian selection protects short word", 1, "if ", "ша ", 1, E2E_VERIFY_SETTLE_DELAY_MS),
            ("short-word protection is consumed once", 1, "if hello ", "if hello ", 0, X11_E2E_CONTEXT_RESOLUTION_VERIFY_DELAY_MS),
            ("context resolves a short word with the next word", 0, "e 'njuj ", "у этого ", 1, X11_E2E_CONTEXT_RESOLUTION_VERIFY_DELAY_MS),
            ("return to EN before internal punctuation", 1, "hello ", "hello ", 0, E2E_VERIFY_SETTLE_DELAY_MS),
            ("learned boundaries keep the internal comma key", 0, "ghj,ktvf ", "проблема ", 1, E2E_VERIFY_SETTLE_DELAY_MS),
            ("return to EN before isolated punctuation", 1, "hello ", "hello ", 0, E2E_VERIFY_SETTLE_DELAY_MS),
            ("isolated dot stays literal after idle", 0, ".", ".", 0, X11_E2E_IDLE_PAUSE_VERIFY_DELAY_MS),
        )
        for name, group, physical, expected_text, expected_group, verify_delay_ms in cases:
            backend.switch_group(group)
            entry.grab_focus()
            pump_gtk()
            kbd.clear_field()
            wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
            kbd.type_physical(physical)
            sleep_pumping(verify_delay_ms / MILLISECONDS_PER_SECOND)
            wait_until(
                lambda: entry.get_text() == expected_text and backend.current_group() == expected_group,
                E2E_VERIFY_GRACE_MS / MILLISECONDS_PER_SECOND,
            )
            actual_text = entry.get_text()
            actual_group = backend.current_group()
            print(f"case={name!r} text={actual_text!r} group={actual_group} expected={expected_text!r}/{expected_group}")
            if actual_text != expected_text or actual_group != expected_group:
                raise RuntimeError(
                    f"Case {name!r} failed: got {actual_text!r}/{actual_group}, "
                    f"expected {expected_text!r}/{expected_group} (last_error={engine.snapshot.last_error!r})"
                )
            sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
        print("TYPING_CASES_E2E_OK")

        # Disable manual-switch protection during isolated feature checks, then re-enable for step 9.
        settings.set("detection.respect_manual_layout", False)

        # 2. Windows-parity deferred Enter: Enter is withheld until replacement completes, then delivered once!
        backend.switch_group(0)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        activated_snapshots.clear()
        kbd.type_physical("ghbdtn\n")
        if not wait_until(
            lambda: len(activated_snapshots) == 1 and backend.current_group() == 1,
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(
                f"Deferred Enter failed: snapshots={activated_snapshots!r} text={entry.get_text()!r}"
            )
        if activated_snapshots != ["привет"]:
            raise RuntimeError(
                f"Enter fired before replacement or more than once: {activated_snapshots!r}"
            )
        print("DEFERRED_ENTER_E2E_OK")

        # 3. Rapid two-line deferred Enter + input holding ("ghbdtn<Enter>gjrf<Enter>").
        backend.switch_group(0)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        activated_snapshots.clear()
        kbd.type_physical("ghbdtn\ngjrf\n")
        if not wait_until(
            lambda: activated_snapshots == ["привет", "приветпока"],
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(f"Rapid multi-Enter burst failed: snapshots={activated_snapshots!r}")
        print("RAPID_DEFERRED_BURST_E2E_OK")

        # 4. Hotkey swallowing: Ctrl+Alt+Z undoes without typing 'z'/'я', Ctrl+Alt+P pauses/resumes.
        backend.switch_group(0)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ghbdtn ")
        if not wait_until(
            lambda: entry.get_text() == "привет ",
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(f"Setup before undo hotkey failed: {entry.get_text()!r}")
        kbd.chord((EVDEV_KEY_LEFTCTRL, EVDEV_KEY_LEFTALT), EVDEV_KEY_Z)
        if not wait_until(
            lambda: entry.get_text() == "ghbdtn ",
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(f"Ctrl+Alt+Z undo failed or leaked character: {entry.get_text()!r}")
        kbd.chord((EVDEV_KEY_LEFTCTRL, EVDEV_KEY_LEFTALT), WL_E2E_KEY_P)
        if not wait_until(lambda: not engine.snapshot.enabled, WL_E2E_CAPS_DEADLINE_SECONDS):
            raise RuntimeError("Ctrl+Alt+P did not pause KeySwitch")
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ghbdtn ")
        sleep_pumping(E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        if entry.get_text() != "ghbdtn ":
            raise RuntimeError(f"Paused engine converted text: {entry.get_text()!r}")
        kbd.chord((EVDEV_KEY_LEFTCTRL, EVDEV_KEY_LEFTALT), WL_E2E_KEY_P)
        if not wait_until(lambda: engine.snapshot.enabled, WL_E2E_CAPS_DEADLINE_SECONDS):
            raise RuntimeError("Ctrl+Alt+P did not resume KeySwitch")
        print("HOTKEY_SWALLOW_E2E_OK")

        # 5. Passive pointer click resets partial word buffer (<Pointer>).
        backend.switch_group(0)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ghb")
        wait_until(lambda: entry.get_text() == "ghb", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        click_x, click_y = widget_screen_center(backend, focus.window, window, "entry")
        mouse.click_at(click_x, click_y, screen_extents())
        sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
        after_click = backend.focused_window()
        if after_click is None or after_click.window != focus.window:
            raise RuntimeError(f"The click left the test window: {after_click!r} instead of {focus!r}")
        kbd.type_physical("dtn ")
        sleep_pumping(E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        if entry.get_text().startswith("при"):
            raise RuntimeError(f"Pointer button did not invalidate pre-click prefix: {entry.get_text()!r}")
        print("POINTER_BUFFER_RESET_E2E_OK")

        # 6. Caps Lock + physical keyboard LED forwarding (LED_CAPSL).
        backend.switch_group(0)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.drain_leds()
        kbd.tap_code(EVDEV_KEY_CAPSLOCK)
        led_synced = wait_until(
            lambda: kbd.drain_leds().get(LED_CAPSL) == 1,
            WL_E2E_CAPS_DEADLINE_SECONDS,
        )
        kbd.type_physical("ghbdtn ")
        if not wait_until(
            lambda: entry.get_text() == "ПРИВЕТ ",
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(f"Caps Lock conversion failed: text={entry.get_text()!r} leds={kbd.drain_leds()!r}")
        kbd.tap_code(EVDEV_KEY_CAPSLOCK)
        wait_until(lambda: kbd.drain_leds().get(LED_CAPSL) == 0, WL_E2E_CAPS_DEADLINE_SECONDS)
        if not led_synced:
            raise RuntimeError(f"Compositor LED_CAPSL was not forwarded to physical keyboard: {kbd.drain_leds()!r}")
        print("CAPSLOCK_AND_LED_SYNC_E2E_OK")

        # 7. Dynamic keyboard hotplug via inotify on /dev/input.
        backend.switch_group(0)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        hotplug_kbd = UinputPhysicalKeyboard(
            name="USB Hotplug Keyboard",
            product_id=WL_E2E_HOTPLUG_PRODUCT_ID,
        )
        try:
            if not wait_until(hotplug_kbd.is_exclusively_grabbed, WL_E2E_FOCUS_DEADLINE_SECONDS):
                raise RuntimeError("Hotplugged keyboard was not dynamically grabbed by WaylandInputHelper")
            hotplug_kbd.type_physical("ghbdtn ")
            if not wait_until(
                lambda: entry.get_text() == "привет ",
                (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
            ):
                raise RuntimeError(f"Hotplugged keyboard typing did not convert: {entry.get_text()!r}")
        finally:
            hotplug_kbd.close()
        sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
        print("HOTPLUG_KEYBOARD_E2E_OK")

        # 8. AT-SPI password field protection. The engine reads the field from its own thread while this
        # one pumps GTK; a read from here would wait for the main loop it holds (the window is ours).
        # The focus moves between the fields as a user moves it, with a click: the engine leaves an
        # application's window alone after its password field until a click, Tab, Enter or another window.
        settings.set("detection.context_policy", "assist")
        backend.switch_group(0)
        password_entry.set_text("")
        mouse.click_at(*widget_screen_center(backend, focus.window, window, "password"), screen_extents())
        sleep_pumping(E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ghbdtn ")
        sleep_pumping((E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND)
        if password_entry.get_text() != "ghbdtn " or backend.current_group() != 0:
            raise RuntimeError(
                f"Password field text was converted: {password_entry.get_text()!r}, group {backend.current_group()}"
            )
        print("ATSPI_PASSWORD_PROTECTION_E2E_OK")
        mouse.click_at(*widget_screen_center(backend, focus.window, window, "entry"), screen_extents())
        sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)

        # 9. Double-Pause learning prompt -> Enter -> RuleEditorWindow -> confirm -> focus restore.
        settings.set("detection.respect_manual_layout", True)
        backend.switch_group(0)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        # Spend manual protection on "ok " first.
        kbd.type_physical("ok ")
        wait_until(lambda: entry.get_text() == "ok ", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("hello")
        wait_until(lambda: entry.get_text() == "hello", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.tap_code(EVDEV_KEY_PAUSE)
        kbd.tap_code(EVDEV_KEY_PAUSE)
        if not wait_until(
            lambda: (
                entry.get_text() == "руддщ"
                and backend.current_group() == 1
                and engine.learning_prompt is not None
                and learning_prompt.get_visible()
            ),
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(
                f"Double-Pause learning prompt failed: text={entry.get_text()!r} "
                f"prompt={engine.learning_prompt!r} visible={learning_prompt.get_visible()}"
            )
        anchor = learning_prompt.anchor
        if anchor is not None and anchor.window is not None:
            offer_windows.append(anchor.window)
        kbd.tap_code(EVDEV_KEY_ENTER)
        if not wait_until(
            lambda: len(rule_windows) == 1 and rule_windows[0].current_draft().pattern == "hello",
            WL_E2E_CAPS_DEADLINE_SECONDS,
        ):
            raise RuntimeError(f"Enter on learning prompt did not open RuleEditorWindow: {rule_windows!r}")
        rule_windows[0].confirm()
        if not wait_until(
            lambda: engine.learning.forced_target(0, "hello") == 1 and not engine._input_suspended,
            WL_E2E_CAPS_DEADLINE_SECONDS,
        ):
            raise RuntimeError("Rule confirmation did not persist learned rule")
        backend.switch_group(0)
        entry.grab_focus()
        sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        # First word after manual switch is protected; second applies the learned rule.
        kbd.type_physical("hello ")
        wait_until(lambda: entry.get_text() == "hello ", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("hello ")
        if not wait_until(
            lambda: entry.get_text() == "руддщ " and backend.current_group() == 1,
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(f"Learned rule was not applied: text={entry.get_text()!r}")
        print("LEARNING_PROMPT_AND_RULE_EDITOR_E2E_OK")

        # 10. Menu layout switch + early prefix switch ("ghbd" -> "прив", then "tn " -> "привет ").
        if not engine.select_alternate_group():
            raise RuntimeError("select_alternate_group failed")
        if not wait_until(lambda: backend.current_group() == 0, WL_E2E_CAPS_DEADLINE_SECONDS):
            raise RuntimeError("Menu layout selection did not switch to group 0")
        settings.set("detection.early_switch", True)
        entry.grab_focus()
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        # Spend manual protection on "ok " (neutral), then type "ghbd" slowly.
        kbd.type_physical("ok ")
        wait_until(lambda: entry.get_text() == "ok ", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        correction_modes.clear()
        kbd.type_physical("ghbd", delay_seconds=WL_E2E_SLOW_KEY_DELAY_MS / MILLISECONDS_PER_SECOND)
        if not wait_until(
            lambda: entry.get_text() == "ok прив" and backend.current_group() == 1,
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(f"Early prefix switch mid-word failed: text={entry.get_text()!r}")
        kbd.type_physical("tn ", delay_seconds=WL_E2E_SLOW_KEY_DELAY_MS / MILLISECONDS_PER_SECOND)
        if not wait_until(
            lambda: entry.get_text() == "ok привет " and "early" in correction_modes,
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(
                f"Early prefix completion failed: text={entry.get_text()!r} modes={correction_modes!r}"
            )
        settings.set("detection.early_switch", False)
        print("EARLY_PREFIX_SWITCH_E2E_OK")

        # 11. Screen-lock ungrab / re-grab via systemd-logind LockedHint.
        settings.set("detection.respect_manual_layout", False)
        if set_session_locked_hint(True):
            try:
                if not wait_until(lambda: not kbd.is_exclusively_grabbed(), WL_E2E_FOCUS_DEADLINE_SECONDS):
                    raise RuntimeError("EVIOCGRAB was NOT released when logind LockedHint became true!")
                backend.switch_group(0)
                kbd.clear_field()
                wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
                kbd.type_physical("ghbdtn ")
                sleep_pumping(WL_E2E_LOCK_SETTLE_SECONDS)
                if entry.get_text() != "ghbdtn ":
                    raise RuntimeError(f"KeySwitch intercepted keys while session was locked: {entry.get_text()!r}")
            finally:
                set_session_locked_hint(False)
            if not wait_until(kbd.is_exclusively_grabbed, WL_E2E_FOCUS_DEADLINE_SECONDS):
                raise RuntimeError("EVIOCGRAB was NOT re-acquired after logind LockedHint became false!")
            kbd.clear_field()
            wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
            kbd.type_physical("ghbdtn ")
            if not wait_until(
                lambda: entry.get_text() == "привет ",
                (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
            ):
                raise RuntimeError(f"Conversion did not resume after unlock: {entry.get_text()!r}")
            print("SCREEN_LOCK_UNGRAB_REGRAB_E2E_OK")

        # 12. No document portal activated by KeySwitch: a desktop session may run one from the start.
        if not portal_before and DOCUMENT_PORTAL_NAME in session_bus_names():
            raise RuntimeError("Document portal was unexpectedly activated")

        print("WAYLAND_SOURCE_E2E_OK")
        return 0
    finally:
        set_session_locked_hint(False)
        try:
            if original_group >= 0:
                backend.switch_group(original_group)
        except Exception:
            pass
        engine.stop()
        backend.close()
        kbd.close()
        mouse.close()
        learning_prompt.destroy()
        window.destroy()
        temporary.cleanup()


def dbus_call(destination: str, object_path: str, method: str, *args: str) -> str:
    completed = subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            destination,
            "--object-path",
            object_path,
            "--method",
            method,
            *args,
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=GDBUS_CALL_TIMEOUT_SECONDS,
    )
    if completed.returncode:
        raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
    return completed.stdout.strip()


def stop_process(process: subprocess.Popen[str]) -> tuple[str, str]:
    if process.poll() is None:
        process.send_signal(signal.SIGTERM)
    try:
        return process.communicate(timeout=NATIVE_PACKAGE_E2E_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired:
        process.kill()
        return process.communicate(timeout=NATIVE_PACKAGE_E2E_FORCED_SHUTDOWN_TIMEOUT_SECONDS)


def run_packaged_e2e(binary: Path, expected_desktop: str) -> int:
    verify_unprivileged_uaccess()
    resolved = binary.resolve(strict=True)
    if not os.access(resolved, os.X_OK):
        raise RuntimeError(f"Packaged binary is not executable: {resolved}")

    # 1. Verify `keyswitch-bin --diagnose` in the live Wayland session.
    with tempfile.TemporaryDirectory() as diag_tmp:
        diag_proc = subprocess.run(
            [str(resolved), "--diagnose"],
            env={**os.environ, "KEYSWITCH_DATA_DIR": diag_tmp},
            check=False,
            capture_output=True,
            text=True,
            timeout=WL_E2E_DIAGNOSE_TIMEOUT_SECONDS,
        )
    if diag_proc.returncode != 0:
        raise RuntimeError(
            f"--diagnose exited {diag_proc.returncode}: stdout={diag_proc.stdout!r} stderr={diag_proc.stderr!r}"
        )
    diag = json.loads(diag_proc.stdout)
    # keyswitch --diagnose prints the backend probe's fields at the top level (app.diagnose).
    probe = diag
    if not isinstance(probe, dict) or not probe.get("available") or probe.get("session_type") != "wayland":
        raise RuntimeError(f"Packaged --diagnose did not report available Wayland probe: {diag!r}")
    if expected_desktop and expected_desktop.casefold() not in str(probe.get("display", "")).casefold():
        raise RuntimeError(f"Packaged --diagnose desktop mismatch: {probe!r}")
    print("PACKAGED_DIAGNOSE_WAYLAND_OK")

    DBusGMainLoop(set_as_default=True)
    Gtk.init()

    kbd = UinputPhysicalKeyboard()
    sleep_pumping(WL_E2E_UDEV_GRAB_SETTLE_SECONDS)

    temporary = tempfile.TemporaryDirectory()
    root = Path(temporary.name)
    settings = SettingsStore(root / "config" / "config.json")
    for path, value in (
        ("general.autostart", False),
        ("general.notifications", False),
        ("general.sound", False),
        ("general.keep_history", True),
        ("appearance.show_indicator", True),
        ("appearance.indicator_style", "letters"),
        ("detection.early_switch", False),
    ):
        settings.set(path, value)

    gio_bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    def bus_has_owner(bus_name: str) -> bool:
        reply = gio_bus.call_sync(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            GLib.Variant("(s)", (bus_name,)),
            GLib.VariantType("(b)"),
            Gio.DBusCallFlags.NONE,
            SESSION_BUS_CALL_TIMEOUT_MS,
            None,
        )
        return bool(reply.unpack()[0])

    session_bus = dbus.SessionBus()
    watcher: TestWatcher | None = None
    if not bus_has_owner(WATCHER_BUS_NAME):
        watcher = TestWatcher(session_bus)

    window = TargetApp()
    entry = window.entry
    activated = window.activations
    window.present()
    entry.grab_focus()
    sleep_pumping(WL_E2E_UDEV_GRAB_SETTLE_SECONDS)

    control_backend = WaylandBackend()
    original_group = control_backend.current_group()
    control_backend.switch_group(0)

    env = dict(os.environ)
    env.update(
        {
            "KEYSWITCH_CONFIG_DIR": str(root / "config"),
            "KEYSWITCH_DATA_DIR": str(root / "data"),
            "XDG_CONFIG_HOME": str(root / "xdg-config"),
            "XDG_DATA_HOME": str(root / "xdg-data"),
        }
    )
    process = subprocess.Popen(
        [str(resolved), "--hidden"],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    destination = f"io.github.olegius88.KeySwitch.StatusNotifierItem.p{process.pid}"
    history = HistoryStore(root / "data" / "history.jsonl")

    try:
        if not wait_until(
            lambda: process.poll() is None and bus_has_owner(destination),
            WL_E2E_TRAY_READY_DEADLINE_SECONDS,
        ):
            rc = process.poll()
            out, err = stop_process(process)
            raise RuntimeError(f"Packaged binary failed to register SNI (rc={rc}): out={out!r} err={err!r}")
        if watcher is not None and not watcher.registered:
            raise RuntimeError("StatusNotifierWatcher did not receive RegisterStatusNotifierItem")
        menu_layout = dbus_call(
            destination,
            MENU_PATH,
            f"{MENU_INTERFACE}.GetLayout",
            "0",
            "1",
            "[]",
        )
        if "Настройки KeySwitch" not in menu_layout or "Выход" not in menu_layout:
            raise RuntimeError(f"Unexpected DBusMenu layout: {menu_layout!r}")
        print("PACKAGED_SNI_MENU_OK")

        if not wait_until(kbd.is_exclusively_grabbed, WL_E2E_FOCUS_DEADLINE_SECONDS):
            raise RuntimeError("Packaged --wayland-input-helper did not grab physical keyboard")

        ensure_entry_focused(window, entry, kbd)

        # Warmup + conversion + deferred Enter on packaged binary.
        converted_ok = False
        for _attempt in range(NATIVE_PACKAGE_E2E_WARMUP_ATTEMPTS):
            control_backend.switch_group(0)
            sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
            kbd.clear_field()
            wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
            # Spend manual switch protection on "ok ", then type "ghbdtn<Enter>".
            kbd.type_physical("ok ")
            wait_until(lambda: entry.get_text() == "ok ", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
            activated.clear()
            kbd.type_physical("ghbdtn\n")
            if wait_until(
                lambda: activated == ["ok привет"] and control_backend.current_group() == 1,
                (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
            ):
                converted_ok = True
                break
        if not converted_ok:
            raise RuntimeError(
                f"Packaged conversion + deferred Enter failed: activated={activated!r} text={entry.get_text()!r}"
            )
        if not any(item.replacement == "привет" for item in history.read()):
            raise RuntimeError(f"Packaged history missing 'привет': {history.read()!r}")
        print("PACKAGED_TYPING_AND_DEFERRED_ENTER_OK")

        # Main-process hang safety: SIGSTOP freezes `keyswitch-bin` while `--wayland-input-helper`
        # stays alive; keys must still pass through within the 50 ms watchdog without bricking input!
        control_backend.switch_group(0)
        sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)

        os.kill(process.pid, signal.SIGSTOP)
        try:
            time.sleep(WL_E2E_SIGSTOP_SETTLE_SECONDS)
            t_start = time.monotonic()
            kbd.type_physical("abc")
            if not wait_until(lambda: entry.get_text() == "abc", WL_E2E_WATCHDOG_DEADLINE_SECONDS):
                raise RuntimeError(
                    f"Keyboard froze when main process received SIGSTOP! text={entry.get_text()!r}"
                )
            elapsed_ms = round((time.monotonic() - t_start) * MILLISECONDS_PER_SECOND)
            print(f"SIGSTOP_WATCHDOG_PASSTHROUGH_OK elapsed_ms={elapsed_ms}")
        finally:
            os.kill(process.pid, signal.SIGCONT)

        sleep_pumping(WL_E2E_UDEV_GRAB_SETTLE_SECONDS)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ok ")
        wait_until(lambda: entry.get_text() == "ok ", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ghbdtn ")
        if not wait_until(
            lambda: entry.get_text() == "ok привет ",
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(
                f"Packaged process did not recover cleanly after SIGCONT: text={entry.get_text()!r}"
            )
        print("SIGCONT_RECOVERY_OK")

        # The input helper dies: its grab ends with it, and KeySwitch starts a new one.
        helpers = subprocess.run(
            ["pgrep", "-P", str(process.pid), "-f", "--", "wayland-input-helper"],
            check=False, capture_output=True, text=True,
        ).stdout.split()
        if len(helpers) != 1:
            raise RuntimeError(f"Expected one input helper of the packaged process, found {helpers!r}")
        os.kill(int(helpers[0]), signal.SIGKILL)
        if not wait_until(lambda: not kbd.is_exclusively_grabbed(), WL_E2E_FOCUS_DEADLINE_SECONDS):
            raise RuntimeError("The keyboard stayed grabbed after the input helper was killed")
        if not wait_until(kbd.is_exclusively_grabbed, WL_E2E_FOCUS_DEADLINE_SECONDS):
            raise RuntimeError("KeySwitch did not start a new input helper")
        control_backend.switch_group(0)
        sleep_pumping(E2E_INTER_CASE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.clear_field()
        wait_until(lambda: entry.get_text() == "", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ok ")
        wait_until(lambda: entry.get_text() == "ok ", E2E_VERIFY_SETTLE_DELAY_MS / MILLISECONDS_PER_SECOND)
        kbd.type_physical("ghbdtn ")
        if not wait_until(
            lambda: entry.get_text() == "ok привет ",
            (E2E_VERIFY_SETTLE_DELAY_MS + E2E_VERIFY_GRACE_MS) / MILLISECONDS_PER_SECOND,
        ):
            raise RuntimeError(f"No conversion after the input helper was restarted: text={entry.get_text()!r}")
        print("HELPER_RESTART_OK")

        out, err = stop_process(process)
        if process.returncode != 0:
            raise RuntimeError(f"Packaged binary exited {process.returncode}: out={out!r} err={err!r}")
        if not wait_until(lambda: not kbd.is_exclusively_grabbed(), WL_E2E_FOCUS_DEADLINE_SECONDS):
            raise RuntimeError("Helper process left physical keyboard grabbed after SIGTERM!")
        print("WAYLAND_PACKAGED_E2E_OK")
        return 0
    finally:
        if process.poll() is None:
            stop_process(process)
        try:
            if original_group >= 0:
                control_backend.switch_group(original_group)
        except Exception:
            pass
        control_backend.close()
        kbd.close()
        window.destroy()
        temporary.cleanup()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("source", "packaged"), default="source")
    parser.add_argument("--binary", type=Path, default=Path("/usr/lib/keyswitch/keyswitch-bin"))
    parser.add_argument("--desktop", default="")
    args = parser.parse_args()

    def _timeout_handler(_signum: int, _frame: object) -> None:
        print("WAYLAND_E2E_TIMEOUT", flush=True)
        os._exit(1)

    signal.signal(signal.SIGALRM, _timeout_handler)
    signal.alarm(WL_E2E_TIMEOUT_SECONDS)
    try:
        if args.mode == "packaged":
            return run_packaged_e2e(args.binary, args.desktop)
        return run_source_e2e(args.desktop)
    finally:
        signal.alarm(0)


if __name__ == "__main__":
    raise SystemExit(main())
