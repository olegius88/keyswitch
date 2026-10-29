#!/usr/bin/env python3
"""The application the Wayland E2E suite types into: a GTK4 window of its own process.

KeySwitch treats windows of its own process as its own (settings, the learning prompt): it does
not follow the focus into them nor read their fields. The text under test therefore lives here,
in another process, as in any application the user types into. Commands come as JSON lines on
stdin, every change of state goes out as one JSON line on stdout.
"""

from __future__ import annotations

import json
import sys

import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

from fixture_values.ui import (  # noqa: E402
    E2E_TEST_ENTRY_MARGIN_PIXELS,
    E2E_TEST_WINDOW_HEIGHT_PIXELS,
    E2E_TEST_WINDOW_WIDTH_PIXELS,
)

TARGET_APPLICATION_ID = "io.github.olegius88.KeySwitchWaylandTarget"
TARGET_WINDOW_TITLE = "KeySwitch Wayland E2E target"


def emit(**fields: object) -> None:
    sys.stdout.write(json.dumps(fields, ensure_ascii=False) + "\n")
    sys.stdout.flush()


class Target:
    def __init__(self, application: Gtk.Application) -> None:
        self.application = application
        self.window = Gtk.ApplicationWindow(application=application, title=TARGET_WINDOW_TITLE)
        self.window.set_default_size(E2E_TEST_WINDOW_WIDTH_PIXELS, E2E_TEST_WINDOW_HEIGHT_PIXELS)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.entries: dict[str, Gtk.Entry | Gtk.PasswordEntry] = {
            "entry": Gtk.Entry(placeholder_text="Wayland E2E input"),
            # As GTK 4 applications ask for a password: the field tells AT-SPI and the compositor what it is.
            "password": Gtk.PasswordEntry(placeholder_text="Password input"),
        }
        for name, entry in self.entries.items():
            entry.set_margin_top(E2E_TEST_ENTRY_MARGIN_PIXELS)
            entry.set_margin_start(E2E_TEST_ENTRY_MARGIN_PIXELS)
            entry.set_margin_end(E2E_TEST_ENTRY_MARGIN_PIXELS)
            entry.connect("changed", self._changed, name)
            box.append(entry)
        self.entries["entry"].connect("activate", lambda widget: emit(event="activate", text=widget.get_text()))
        self.window.set_child(box)
        # Where a click of the suite's pointer landed, should it miss the field it aimed at.
        clicks = Gtk.GestureClick()
        clicks.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        clicks.connect("pressed", lambda _gesture, _count, x, y: emit(event="click", x=x, y=y))
        self.window.add_controller(clicks)
        self.window.connect("notify::is-active", lambda window, _spec: emit(event="active", value=window.is_active()))
        display = Gdk.Display.get_default()
        seat = display.get_default_seat() if display is not None else None
        keyboard = seat.get_keyboard() if seat is not None else None
        if keyboard is not None:
            keyboard.connect("notify::caps-lock-state", lambda device, _spec: emit(event="caps", value=device.get_caps_lock_state()))
            emit(event="caps", value=keyboard.get_caps_lock_state())
        channel = GLib.IOChannel.unix_new(sys.stdin.fileno())
        channel.add_watch(GLib.IOCondition.IN | GLib.IOCondition.HUP, self._command)
        self.window.present()

    def _changed(self, entry: Gtk.Entry | Gtk.PasswordEntry, name: str) -> None:
        emit(event="text", widget=name, text=entry.get_text())

    def _command(self, channel: GLib.IOChannel, condition: GLib.IOCondition) -> bool:
        line = sys.stdin.readline()
        if not line:
            self.application.quit()
            return GLib.SOURCE_REMOVE
        command = json.loads(line)
        name = command.get("widget", "entry")
        entry = self.entries[name]
        match command["cmd"]:
            case "present":
                self.window.present()
            case "focus":
                entry.grab_focus()
                entry.set_position(-1)
            case "set_text":
                entry.set_text(command["text"])
                entry.set_position(-1)
            case "bounds":
                ok, bounds = entry.compute_bounds(self.window)
                emit(event="bounds", widget=name, ok=ok, x=bounds.get_x(), y=bounds.get_y(),
                     width=bounds.get_width(), height=bounds.get_height())
            case "quit":
                self.application.quit()
            case _:
                pass
        emit(event="done", cmd=command["cmd"], seq=command.get("seq", 0))
        return GLib.SOURCE_CONTINUE


def main() -> int:
    application = Gtk.Application(application_id=TARGET_APPLICATION_ID)
    holder: list[Target] = []
    application.connect("activate", lambda app: holder.append(Target(app)))
    return application.run([sys.argv[0]])


if __name__ == "__main__":
    raise SystemExit(main())
