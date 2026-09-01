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
from keyswitch.engine import KeySwitchEngine, LearningPrompt
from keyswitch.history import HistoryStore
from keyswitch.learning_prompt import LearningPromptWindow
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

    def tap_keysym(self, keysym: int) -> None:
        keycode = int(self.libraries.x11.XKeysymToKeycode(self.display, keysym))
        if not keycode:
            raise RuntimeError(f"No X11 keycode for keysym {keysym:#x}")
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
    application = Gtk.Application(
        application_id="io.github.olegius88.KeySwitchSourceE2E"
    )
    application.register(None)
    original_group = -1
    result = E2EResult()
    cases = (
        ("EN pause correction", 0, "ghbdtn", "привет", 1, 2300),
        ("RU keys to English", 1, "hello ", "hello ", 0, 900),
        ("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1, 900),
        ("return to EN before punctuation test", 1, "hello ", "hello ", 0, 900),
        ("punctuation boundary keeps its glyph", 0, "ghbdtn,", "привет,", 1, 900),
        ("manual layout switch protects next word", 0, "ghbdtn ", "ghbdtn ", 0, 900),
        ("manual protection is consumed once", 0, "ghbdtn ", "привет ", 1, 900),
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
        (
            _name,
            group,
            physical,
            _expected_text,
            _expected_group,
            verify_delay,
        ) = cases[index]
        backend.switch_group(group)
        backend._libraries.x11.XSync(backend._control, 0)
        entry.set_text("")
        entry.grab_focus()
        typer.type(physical)
        GLib.timeout_add(verify_delay, verify_case, index)
        return GLib.SOURCE_REMOVE

    def verify_case(index: int) -> bool:
        name, _group, _physical, expected_text, expected_group, _delay = cases[index]
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
        GLib.timeout_add(200, start_learning_case)
        return GLib.SOURCE_REMOVE

    def start_learning_case() -> bool:
        backend.switch_group(0)
        backend._libraries.x11.XSync(backend._control, 0)
        entry.set_text("")
        entry.grab_focus()
        typer.type("hello")
        typer.tap_keysym(0xFF13)
        GLib.timeout_add(900, verify_learning_prompt)
        return GLib.SOURCE_REMOVE

    def verify_learning_prompt() -> bool:
        prompt = engine.learning_prompt
        if (
            entry.get_text() != "руддщ"
            or backend.current_group() != 1
            or prompt is None
            or not learning_prompt.get_visible()
        ):
            print(
                "learning_prompt_failed "
                f"text={entry.get_text()!r} group={backend.current_group()} "
                f"prompt={prompt!r} visible={learning_prompt.get_visible()}"
            )
            print("E2E_FAILED")
            loop.quit()
            return GLib.SOURCE_REMOVE
        typer.tap_keysym(0xFF0D)
        GLib.timeout_add(500, verify_learning_confirmation)
        return GLib.SOURCE_REMOVE

    def verify_learning_confirmation() -> bool:
        required = int(settings.get("detection.learning_confirmations", 2))
        if (
            engine.learning_prompt is not None
            or engine.learning.forced_target(0, "hello", required) != 1
            or learning_prompt.get_visible()
        ):
            print("learning_confirmation_failed")
            print("E2E_FAILED")
            loop.quit()
            return GLib.SOURCE_REMOVE
        backend.switch_group(0)
        backend._libraries.x11.XSync(backend._control, 0)
        entry.set_text("")
        entry.grab_focus()
        typer.type("hello ")
        GLib.timeout_add(900, verify_learned_rule)
        return GLib.SOURCE_REMOVE

    def verify_learned_rule() -> bool:
        if entry.get_text() != "руддщ " or backend.current_group() != 1:
            print(
                f"learned_rule_failed text={entry.get_text()!r} "
                f"group={backend.current_group()}"
            )
            print("E2E_FAILED")
            loop.quit()
            return GLib.SOURCE_REMOVE
        entries = history.read()
        expected_history = [
            ("ghbdtn", "привет"),
            ("руддщ", "hello"),
            (",fpf", "база"),
            ("руддщ", "hello"),
            ("ghbdtn", "привет"),
            ("ghbdtn", "привет"),
            ("hello", "руддщ"),
        ]
        actual_history = [(item.original, item.replacement) for item in entries]
        print(f"history={actual_history!r}")
        if actual_history != expected_history:
            print("E2E_FAILED")
            loop.quit()
            return GLib.SOURCE_REMOVE
        if not engine.select_alternate_group():
            print(f"menu_layout_queue_failed error={engine.snapshot.last_error!r}")
            print("E2E_FAILED")
            loop.quit()
            return GLib.SOURCE_REMOVE
        GLib.timeout_add(300, verify_menu_layout_selection)
        return GLib.SOURCE_REMOVE

    def verify_menu_layout_selection() -> bool:
        actual_group = backend.current_group()
        if actual_group != 0 or engine.snapshot.current_group != 0:
            print(
                "menu_layout_failed "
                f"backend_group={actual_group} "
                f"engine_group={engine.snapshot.current_group}"
            )
            print("E2E_FAILED")
            loop.quit()
            return GLib.SOURCE_REMOVE
        print("MENU_LAYOUT_SELECTION_E2E_OK")
        print("E2E_OK")
        result.exit_code = 0
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
        learning_prompt.destroy()
        window.destroy()
        temporary.cleanup()
    return result.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
