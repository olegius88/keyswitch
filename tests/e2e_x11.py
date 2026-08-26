#!/usr/bin/env python3
"""Real X11 end-to-end test: GTK entry -> RECORD -> detector -> XTEST."""

from __future__ import annotations

import ctypes
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.x11_backend import KeyEvent, X11Backend, _Libraries


@dataclass
class E2EResult:
    exit_code: int = 1
    observed: list[tuple[str, str, int]] = field(default_factory=list)
    events: int = 0
    sample: list[tuple[str, str, tuple[str, ...], int]] = field(
        default_factory=list
    )


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
    original_group = -1
    result = E2EResult()
    cases = (
        ("EN keys to Russian", 0, "ghbdtn ", "привет ", 1),
        ("RU keys to English", 1, "hello ", "hello ", 0),
        ("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1),
        ("punctuation boundary keeps its glyph", 0, "ghbdtn,", "привет,", 1),
    )

    def abort_on_timeout() -> bool:
        print("E2E_TIMEOUT")
        loop.quit()
        return GLib.SOURCE_REMOVE

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
    engine_listener = backend._listener

    def observe(event: KeyEvent) -> None:
        result.events += 1
        if event.pressed and len(result.sample) < 12:
            result.sample.append(
                (event.key_name, event.character, event.characters, event.group)
            )
        assert engine_listener is not None
        engine_listener(event)

    backend._listener = observe
    original_group = backend.current_group()
    GLib.timeout_add_seconds(20, abort_on_timeout)

    def type_case(index: int) -> bool:
        _name, group, physical, _expected_text, _expected_group = cases[index]
        backend.switch_group(group)
        backend._libraries.x11.XSync(backend._control, 0)
        entry.set_text("")
        entry.grab_focus()
        typer.type(physical)
        GLib.timeout_add(900, verify_case, index)
        return GLib.SOURCE_REMOVE

    def verify_case(index: int) -> bool:
        name, _group, _physical, expected_text, expected_group = cases[index]
        actual_text = entry.get_text()
        actual_group = backend.current_group()
        result.observed.append((name, actual_text, actual_group))
        print(
            f"case={name!r} text={actual_text!r} group={actual_group} "
            f"expected={expected_text!r}/{expected_group} events={result.events} "
            f"backend_running={backend.running} engine={engine.snapshot.last_action!r}"
        )
        if actual_text != expected_text or actual_group != expected_group:
            print(f"event_sample={result.sample!r}")
            print("E2E_FAILED")
            loop.quit()
            return GLib.SOURCE_REMOVE
        if index + 1 < len(cases):
            GLib.timeout_add(200, type_case, index + 1)
            return GLib.SOURCE_REMOVE
        entries = history.read()
        expected_history = [
            ("ghbdtn", "привет"),
            ("руддщ", "hello"),
            (",fpf", "база"),
            ("ghbdtn", "привет"),
        ]
        actual_history = [(item.original, item.replacement) for item in entries]
        ok = actual_history == expected_history
        print(f"history={actual_history!r}")
        print("E2E_OK" if ok else "E2E_FAILED")
        result.exit_code = 0 if ok else 1
        loop.quit()
        return GLib.SOURCE_REMOVE

    GLib.timeout_add(450, type_case, 0)
    try:
        loop.run()
    finally:
        if original_group >= 0:
            backend.switch_group(original_group)
            backend._libraries.x11.XSync(backend._control, 0)
        engine.stop()
        typer.close()
        window.destroy()
        temporary.cleanup()
    return result.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
