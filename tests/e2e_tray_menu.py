#!/usr/bin/env python3
"""Exercise the StatusNotifierItem and DBusMenu objects on a real session bus."""

from __future__ import annotations

import argparse
import select
import subprocess
import sys
import threading
from pathlib import Path
from typing import cast

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

from keyswitch.tray import (
    ITEM_INTERFACE,
    MENU_INTERFACE,
    MENU_PATH,
    MENU_SETTINGS,
    MENU_SWITCH_LAYOUT,
    OBJECT_PATH,
    StatusNotifierItem,
)


WATCHER_BUS_NAME = "org.kde.StatusNotifierWatcher"
WATCHER_INTERFACE = "org.kde.StatusNotifierWatcher"
WATCHER_PATH = "/StatusNotifierWatcher"


class TestWatcher(dbus.service.Object):
    """Minimal watcher used only to accept the item's real registration call."""

    def __init__(self, bus: dbus.Bus) -> None:
        self._bus_name = dbus.service.BusName(WATCHER_BUS_NAME, bus)
        super().__init__(self._bus_name, WATCHER_PATH)

    @dbus.service.method(WATCHER_INTERFACE, in_signature="s", out_signature="")
    def RegisterStatusNotifierItem(self, service: str) -> None:
        print(f"REGISTERED {service}", flush=True)


def run_watcher() -> int:
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    watcher = TestWatcher(bus)
    loop = GLib.MainLoop()
    def stop_loop() -> bool:
        loop.quit()
        return GLib.SOURCE_REMOVE

    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, 15, stop_loop)
    print("READY", flush=True)
    loop.run()
    watcher.remove_from_connection()
    return 0


def read_line(process: subprocess.Popen[str], timeout: float) -> str:
    assert process.stdout is not None
    readable, _, _ = select.select([process.stdout], [], [], timeout)
    if not readable:
        raise RuntimeError("StatusNotifierWatcher did not answer in time")
    line = cast(str, process.stdout.readline().strip())
    if not line:
        stderr = process.stderr.read().strip() if process.stderr else ""
        raise RuntimeError(
            f"StatusNotifierWatcher exited before answering: {stderr or process.poll()}"
        )
    return line


def dbus_call(destination: str, object_path: str, method: str, *arguments: str) -> str:
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
            *arguments,
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    if completed.returncode:
        raise RuntimeError(completed.stderr.strip() or completed.stdout.strip())
    return completed.stdout.strip()


def run_probe() -> int:
    watcher = subprocess.Popen(
        [sys.executable, str(Path(__file__).resolve()), "--watcher"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    item: StatusNotifierItem | None = None
    loop: GLib.MainLoop | None = None
    loop_thread: threading.Thread | None = None
    try:
        if read_line(watcher, 5) != "READY":
            raise RuntimeError("StatusNotifierWatcher returned an invalid greeting")

        DBusGMainLoop(set_as_default=True)
        settings_opened = threading.Event()
        layout_switched = threading.Event()
        item = StatusNotifierItem(
            settings_opened.set,
            lambda: None,
            Path(__file__).resolve().parents[1] / "src/keyswitch/resources",
            on_switch_layout=layout_switched.set,
            on_sound_toggle=lambda: None,
            on_notifications_toggle=lambda: None,
            on_history=lambda: None,
            on_exceptions=lambda: None,
            on_about=lambda: None,
            on_quit=lambda: None,
        )
        item.set_layout(1)
        if read_line(watcher, 5) != f"REGISTERED {OBJECT_PATH}":
            raise RuntimeError("StatusNotifierItem registered an unexpected object path")

        loop = GLib.MainLoop()
        loop_thread = threading.Thread(target=loop.run, daemon=True)
        loop_thread.start()
        destination = item._bus_name.get_name()

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
        version = dbus_call(
            destination,
            MENU_PATH,
            "org.freedesktop.DBus.Properties.Get",
            MENU_INTERFACE,
            "Version",
        )
        layout = dbus_call(
            destination,
            MENU_PATH,
            f"{MENU_INTERFACE}.GetLayout",
            "0",
            "1",
            "[]",
        )
        dbus_call(
            destination,
            MENU_PATH,
            f"{MENU_INTERFACE}.Event",
            str(MENU_SETTINGS),
            "clicked",
            "<''>",
            "0",
        )
        dbus_call(
            destination,
            MENU_PATH,
            f"{MENU_INTERFACE}.Event",
            str(MENU_SWITCH_LAYOUT),
            "clicked",
            "<''>",
            "0",
        )

        checks = {
            "ItemIsMenu": "true" in item_is_menu,
            "Menu object path": MENU_PATH in menu_path,
            "DBusMenu version": "uint32 3" in version,
            "settings menu item": "Настройки KeySwitch" in layout,
            "alternate layout menu item": "Переключить на английский (EN)" in layout,
            "autoswitch menu item": "Автопереключение" in layout,
            "quit menu item": "Выход" in layout,
            "clicked event": settings_opened.wait(timeout=2),
            "layout clicked event": layout_switched.wait(timeout=2),
        }
        failed = [name for name, passed in checks.items() if not passed]
        if failed:
            raise RuntimeError(f"Failed tray checks: {', '.join(failed)}")
        print("TRAY_MENU_E2E_OK")
        return 0
    finally:
        if item is not None:
            item.close()
        if loop is not None:
            loop.quit()
        if loop_thread is not None:
            loop_thread.join(timeout=2)
        watcher.terminate()
        try:
            watcher.wait(timeout=3)
        except subprocess.TimeoutExpired:
            watcher.kill()
            watcher.wait(timeout=3)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--watcher", action="store_true")
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    return run_watcher() if arguments.watcher else run_probe()


if __name__ == "__main__":
    raise SystemExit(main())
