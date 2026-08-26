#!/usr/bin/env python3
"""Real X11 end-to-end test: GTK entry -> RECORD -> detector -> XTEST."""

from __future__ import annotations

import ctypes
import tempfile
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.x11_backend import X11Backend, _Libraries


class PhysicalTyper:
    def __init__(self) -> None:
        self.libraries = _Libraries()
        self.display = self.libraries.x11.XOpenDisplay(None)
        if not self.display:
            raise RuntimeError("Cannot open DISPLAY for E2E typing")

    def type(self, physical_keys: str) -> None:
        for character in physical_keys:
            keysym = ord(character)
            keycode = int(self.libraries.x11.XKeysymToKeycode(self.display, keysym))
            if not keycode:
                raise RuntimeError(f"No X11 keycode for {character!r}")
            self.libraries.xtst.XTestFakeKeyEvent(self.display, keycode, 1, 18)
            self.libraries.xtst.XTestFakeKeyEvent(self.display, keycode, 0, 8)
        self.libraries.x11.XSync(self.display, 0)

    def close(self) -> None:
        if self.display:
            self.libraries.x11.XCloseDisplay(self.display)
            self.display = None


def main() -> int:
    Gtk.init()
    temporary = tempfile.TemporaryDirectory()
    root = Path(temporary.name)
    settings = SettingsStore(root / "config.json")
    settings.set("general.keep_history", True)
    history = HistoryStore(root / "history.jsonl")
    backend = X11Backend()
    engine = KeySwitchEngine(settings, history, backend)
    typer = PhysicalTyper()
    loop = GLib.MainLoop()
    result = {"exit": 1, "first": "", "second": ""}

    window = Gtk.Window(title="KeySwitch E2E")
    window.set_default_size(520, 120)
    entry = Gtk.Entry(placeholder_text="E2E input")
    entry.set_margin_top(24)
    entry.set_margin_bottom(24)
    entry.set_margin_start(24)
    entry.set_margin_end(24)
    window.set_child(entry)
    window.present()
    entry.grab_focus()
    engine.start()

    def type_first() -> bool:
        backend.switch_group(0)
        backend._libraries.x11.XSync(backend._control, 0)
        entry.set_text("")
        entry.grab_focus()
        typer.type("ghbdtn ")
        GLib.timeout_add(900, verify_first)
        return GLib.SOURCE_REMOVE

    def verify_first() -> bool:
        result["first"] = entry.get_text()
        entry.set_text("")
        backend.switch_group(1)
        backend._libraries.x11.XSync(backend._control, 0)
        entry.grab_focus()
        typer.type("hello ")
        GLib.timeout_add(900, verify_second)
        return GLib.SOURCE_REMOVE

    def verify_second() -> bool:
        result["second"] = entry.get_text()
        entries = history.read()
        ok = (
            result["first"] == "привет "
            and result["second"] == "hello "
            and len(entries) == 2
            and backend.current_group() == 0
        )
        print(f"first={result['first']!r}")
        print(f"second={result['second']!r}")
        print(f"history={[(item.original, item.replacement) for item in entries]!r}")
        print(f"final_group={backend.current_group()}")
        print("E2E_OK" if ok else "E2E_FAILED")
        result["exit"] = 0 if ok else 1
        loop.quit()
        return GLib.SOURCE_REMOVE

    GLib.timeout_add(450, type_first)
    try:
        loop.run()
    finally:
        engine.stop()
        typer.close()
        window.destroy()
        temporary.cleanup()
    return int(result["exit"])


if __name__ == "__main__":
    raise SystemExit(main())
