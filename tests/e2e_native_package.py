#!/usr/bin/env python3
"""Exercise the packaged native executable through real X11 and D-Bus."""

from __future__ import annotations

import argparse
import ctypes
import os
import signal
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

import dbus
import dbus.service
import gi
from dbus.mainloop.glib import DBusGMainLoop

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

from keyswitch.config import SettingsStore
from keyswitch.history import HistoryStore
from keyswitch.learning import LearningStore
from keyswitch.tray import ITEM_INTERFACE, MENU_INTERFACE, MENU_PATH, OBJECT_PATH
from keyswitch.x11_backend import XKB_USE_CORE_KBD, XkbStateRec, _Libraries


WATCHER_BUS_NAME = "org.kde.StatusNotifierWatcher"
WATCHER_INTERFACE = "org.kde.StatusNotifierWatcher"
WATCHER_PATH = "/StatusNotifierWatcher"


class TestWatcher(dbus.service.Object):
    """Minimal host required for the packaged StatusNotifierItem."""

    def __init__(self, bus: dbus.Bus) -> None:
        self.registered = False
        self.service = ""
        self._bus_name = dbus.service.BusName(WATCHER_BUS_NAME, bus)
        super().__init__(self._bus_name, WATCHER_PATH)

    @dbus.service.method(WATCHER_INTERFACE, in_signature="s", out_signature="")
    def RegisterStatusNotifierItem(self, service: str) -> None:
        self.service = service
        self.registered = True


class PhysicalTyper:
    """Send physical US-layout keys without depending on an input method."""

    def __init__(self) -> None:
        self.libraries = _Libraries()
        self.display = self.libraries.x11.XOpenDisplay(None)
        if not self.display:
            raise RuntimeError("Cannot open DISPLAY for native E2E typing")

    def type(self, physical_keys: str) -> None:
        for character in physical_keys:
            keysym = ord(character)
            keycode = int(self.libraries.x11.XKeysymToKeycode(self.display, keysym))
            if not keycode:
                raise RuntimeError(f"No X11 keycode for {character!r}")
            self.libraries.xtst.XTestFakeKeyEvent(self.display, keycode, 1, 18)
            self.libraries.xtst.XTestFakeKeyEvent(self.display, keycode, 0, 8)
        self.libraries.x11.XSync(self.display, 0)

    def tap_keysym(self, keysym: int) -> None:
        keycode = int(self.libraries.x11.XKeysymToKeycode(self.display, keysym))
        if not keycode:
            raise RuntimeError(f"No X11 keycode for keysym {keysym:#x}")
        self.libraries.xtst.XTestFakeKeyEvent(self.display, keycode, 1, 18)
        self.libraries.xtst.XTestFakeKeyEvent(self.display, keycode, 0, 8)
        self.libraries.x11.XSync(self.display, 0)

    def current_group(self) -> int:
        state = XkbStateRec()
        result = self.libraries.x11.XkbGetState(
            self.display, XKB_USE_CORE_KBD, ctypes.byref(state)
        )
        if result != 0:
            raise RuntimeError("Cannot read the current XKB group")
        return int(state.group)

    def switch_group(self, group: int) -> None:
        if not self.libraries.x11.XkbLockGroup(
            self.display, XKB_USE_CORE_KBD, group
        ):
            raise RuntimeError(f"Cannot switch the XKB group to {group}")
        self.libraries.x11.XSync(self.display, 0)

    def close(self) -> None:
        if self.display:
            self.libraries.x11.XCloseDisplay(self.display)
            self.display = None


@dataclass
class NativeResult:
    exit_code: int = 1
    observed: list[tuple[str, str, int]] = field(default_factory=list)


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
        timeout=10,
    )
    if completed.returncode:
        raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
    return completed.stdout.strip()


def stop_process(process: subprocess.Popen[str]) -> tuple[str, str]:
    """Request a GLib shutdown, escalating only for a stuck test process."""

    if process.poll() is None:
        process.send_signal(signal.SIGTERM)
    try:
        stdout, stderr = process.communicate(timeout=8)
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate(timeout=3)
    return stdout, stderr


def configure_isolated_profile(root: Path) -> None:
    settings = SettingsStore(root / "config" / "config.json")
    for path, value in (
        ("general.autostart", False),
        ("general.notifications", False),
        ("general.sound", False),
        ("general.keep_history", True),
        ("appearance.show_indicator", True),
        ("appearance.indicator_style", "letters"),
        # Synthetic typing has no inter-key gaps; prefix switching is covered
        # by unit tests and the source E2E and would race with the burst here.
        ("detection.early_switch", False),
    ):
        settings.set(path, value)


def main() -> int:
    arguments = build_parser().parse_args()
    binary = arguments.binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"Native executable is unavailable: {binary}")

    DBusGMainLoop(set_as_default=True)
    Gtk.init()
    temporary = tempfile.TemporaryDirectory()
    root = Path(temporary.name)
    configure_isolated_profile(root)
    environment = os.environ.copy()
    environment.update(
        {
            "KEYSWITCH_CONFIG_DIR": str(root / "config"),
            "KEYSWITCH_DATA_DIR": str(root / "data"),
            "XDG_CONFIG_HOME": str(root / "xdg-config"),
            "XDG_DATA_HOME": str(root / "xdg-data"),
            "GTK_A11Y": "none",
        }
    )

    watcher = TestWatcher(dbus.SessionBus())
    process = subprocess.Popen(
        [str(binary), "--hidden"],
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    destination = f"io.github.olegius88.KeySwitch.StatusNotifierItem.p{process.pid}"
    history = HistoryStore(root / "data" / "history.jsonl")
    learning = LearningStore(root / "data" / "learning.json")
    typer = PhysicalTyper()
    result = NativeResult()
    loop = GLib.MainLoop()
    original_group = typer.current_group()
    cases = (
        ("EN pause correction", 0, "ghbdtn", "привет", 1, 2300),
        ("RU keys to English", 1, "hello ", "hello ", 0, 1000),
        ("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1, 1000),
        ("return to EN before punctuation test", 1, "hello ", "hello ", 0, 1000),
        ("punctuation boundary keeps its glyph", 0, "ghbdtn,", "привет,", 1, 1000),
        ("manual layout switch protects next word", 0, "ghbdtn ", "ghbdtn ", 0, 1000),
        ("manual protection is consumed once", 0, "ghbdtn ", "привет ", 1, 1000),
        ("short Russian word switches to English", 1, "if ", "if ", 0, 1000),
        ("manual Russian selection protects short word", 1, "if ", "ша ", 1, 1000),
        ("short-word protection is consumed once", 1, "if ", "if ", 0, 1000),
    )
    window = Gtk.Window(title="KeySwitch native package E2E")
    window.set_default_size(520, 120)
    entry = Gtk.Entry(placeholder_text="Native package E2E input")
    entry.set_margin_top(24)
    entry.set_margin_bottom(24)
    entry.set_margin_start(24)
    entry.set_margin_end(24)
    window.set_child(entry)
    window.present()
    entry.grab_focus()

    def abort_on_timeout() -> bool:
        print("NATIVE_E2E_TIMEOUT")
        loop.quit()
        return GLib.SOURCE_REMOVE

    def fail(message: str) -> bool:
        print(f"NATIVE_E2E_FAILED: {message}")
        loop.quit()
        return GLib.SOURCE_REMOVE

    def wait_for_application() -> bool:
        return_code = process.poll()
        if return_code is not None:
            return fail(f"native process exited early with {return_code}")
        if not watcher.registered:
            return GLib.SOURCE_CONTINUE
        if watcher.service != OBJECT_PATH:
            return fail(f"unexpected registered tray path {watcher.service!r}")
        try:
            item_is_menu = dbus_call(
                destination,
                OBJECT_PATH,
                "org.freedesktop.DBus.Properties.Get",
                ITEM_INTERFACE,
                "ItemIsMenu",
            )
            menu_path = dbus_call(
                destination,
                OBJECT_PATH,
                "org.freedesktop.DBus.Properties.Get",
                ITEM_INTERFACE,
                "Menu",
            )
            layout = dbus_call(
                destination,
                MENU_PATH,
                f"{MENU_INTERFACE}.GetLayout",
                "0",
                "1",
                "[]",
            )
        except (OSError, RuntimeError, subprocess.SubprocessError) as error:
            return fail(f"cannot query packaged tray menu: {error}")
        checks = {
            "ItemIsMenu": "true" in item_is_menu,
            "Menu path": MENU_PATH in menu_path,
            "Settings": "Настройки KeySwitch" in layout,
            "Autoswitch": "Автопереключение" in layout,
            "History": "История исправлений" in layout,
            "Exclusions": "Программы-исключения" in layout,
            "Quit": "Выход" in layout,
        }
        failed = [name for name, passed in checks.items() if not passed]
        if failed:
            return fail(f"packaged tray checks failed: {', '.join(failed)}")
        GLib.timeout_add(300, type_case, 0)
        return GLib.SOURCE_REMOVE

    def type_case(index: int) -> bool:
        (
            name,
            group,
            physical,
            _expected_text,
            _expected_group,
            verify_delay,
        ) = cases[index]
        try:
            typer.switch_group(group)
            entry.set_text("")
            entry.grab_focus()
            typer.type(physical)
        except (OSError, RuntimeError) as error:
            return fail(f"cannot type {name!r}: {error}")
        GLib.timeout_add(verify_delay, verify_case, index)
        return GLib.SOURCE_REMOVE

    def verify_case(index: int) -> bool:
        name, _group, _physical, expected_text, expected_group, _delay = cases[index]
        actual_text = entry.get_text()
        actual_group = typer.current_group()
        result.observed.append((name, actual_text, actual_group))
        print(
            f"case={name!r} text={actual_text!r} group={actual_group} "
            f"expected={expected_text!r}/{expected_group}"
        )
        if actual_text != expected_text or actual_group != expected_group:
            return fail(f"wrong correction in {name!r}")
        if index + 1 < len(cases):
            GLib.timeout_add(200, type_case, index + 1)
            return GLib.SOURCE_REMOVE
        actual_history = [
            (item.original, item.replacement) for item in history.read()
        ]
        expected_history = [
            ("ghbdtn", "привет"),
            ("руддщ", "hello"),
            (",fpf", "база"),
            ("руддщ", "hello"),
            ("ghbdtn", "привет"),
            ("ghbdtn", "привет"),
            ("ша", "if"),
            ("ша", "if"),
        ]
        if actual_history != expected_history:
            return fail(f"wrong correction history: {actual_history!r}")
        GLib.timeout_add(200, start_learning_case)
        return GLib.SOURCE_REMOVE

    def start_learning_case() -> bool:
        try:
            typer.switch_group(0)
            entry.set_text("")
            window.present()
            entry.grab_focus()
            typer.type("hello")
            typer.tap_keysym(0xFF13)
        except (OSError, RuntimeError) as error:
            return fail(f"cannot start packaged learning scenario: {error}")
        GLib.timeout_add(900, confirm_learning_prompt)
        return GLib.SOURCE_REMOVE

    def confirm_learning_prompt() -> bool:
        if entry.get_text() != "руддщ" or typer.current_group() != 1:
            return fail(
                "manual learning conversion failed: "
                f"text={entry.get_text()!r} group={typer.current_group()}"
            )
        try:
            typer.tap_keysym(0xFF0D)
        except RuntimeError as error:
            return fail(f"cannot confirm packaged learning prompt: {error}")
        GLib.timeout_add(500, verify_learning_confirmation)
        return GLib.SOURCE_REMOVE

    def verify_learning_confirmation() -> bool:
        required = 2
        learning.load()
        if learning.forced_target(0, "hello", required) != 1:
            return fail("Enter did not persist the packaged learning rule")
        try:
            typer.switch_group(0)
            entry.set_text("")
            window.present()
            entry.grab_focus()
            typer.type("hello ")
        except (OSError, RuntimeError) as error:
            return fail(f"cannot type the learned packaged word: {error}")
        GLib.timeout_add(900, verify_manual_override_of_learned_rule)
        return GLib.SOURCE_REMOVE

    def verify_manual_override_of_learned_rule() -> bool:
        if entry.get_text() != "hello " or typer.current_group() != 0:
            return fail(
                "manual layout did not override the packaged learned rule: "
                f"text={entry.get_text()!r} group={typer.current_group()}"
            )
        try:
            entry.set_text("")
            window.present()
            entry.grab_focus()
            typer.type("hello ")
        except (OSError, RuntimeError) as error:
            return fail(f"cannot repeat the learned packaged word: {error}")
        GLib.timeout_add(900, verify_learned_rule)
        return GLib.SOURCE_REMOVE

    def verify_learned_rule() -> bool:
        if entry.get_text() != "руддщ " or typer.current_group() != 1:
            return fail(
                "packaged learned rule did not run: "
                f"text={entry.get_text()!r} group={typer.current_group()}"
            )
        actual_history = [
            (item.original, item.replacement) for item in history.read()
        ]
        expected_history = [
            ("ghbdtn", "привет"),
            ("руддщ", "hello"),
            (",fpf", "база"),
            ("руддщ", "hello"),
            ("ghbdtn", "привет"),
            ("ghbdtn", "привет"),
            ("ша", "if"),
            ("ша", "if"),
            ("hello", "руддщ"),
        ]
        if actual_history != expected_history:
            return fail(f"wrong learned correction history: {actual_history!r}")
        result.exit_code = 0
        print("NATIVE_LEARNING_PROMPT_E2E_OK")
        print("NATIVE_E2E_OK")
        loop.quit()
        return GLib.SOURCE_REMOVE

    GLib.timeout_add_seconds(50, abort_on_timeout)
    GLib.timeout_add(100, wait_for_application)
    captured_stdout = ""
    captured_stderr = ""
    captured_log = ""
    try:
        loop.run()
    finally:
        if original_group >= 0:
            typer.switch_group(original_group)
        captured_stdout, captured_stderr = stop_process(process)
        log_path = root / "data" / "keyswitch.log"
        if log_path.is_file():
            try:
                captured_log = log_path.read_text(encoding="utf-8")
            except OSError as error:
                captured_log = f"Cannot read native log: {error}"
        typer.close()
        window.destroy()
        watcher.remove_from_connection()
        temporary.cleanup()
    if result.exit_code and (captured_stdout or captured_stderr):
        print("native stdout:", captured_stdout.strip())
        print("native stderr:", captured_stderr.strip())
    if result.exit_code and captured_log:
        print("native log:", captured_log.strip())
    if result.exit_code == 0 and process.returncode != 0:
        print(f"NATIVE_E2E_FAILED: native shutdown returned {process.returncode}")
        return 1
    return result.exit_code


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    return parser


if __name__ == "__main__":
    raise SystemExit(main())
