"""Focused Linux prompt for explicitly confirming a learned layout rule."""

from __future__ import annotations

from collections.abc import Callable
from typing import Protocol, cast

import gi

gi.require_version("Atspi", "2.0")
gi.require_version("Gdk", "4.0")
gi.require_version("GdkX11", "4.0")
gi.require_version("Gtk", "4.0")

from gi.repository import Atspi, Gdk, GdkX11, GLib, Gtk  # noqa: E402

from .backend import ScreenAnchor
from .engine import LearningPrompt


class PromptBackend(Protocol):
    def input_anchor(self) -> ScreenAnchor | None: ...

    def position_window(self, window: int, x: int, y: int) -> bool: ...

    def restore_window(self, window: int | None) -> bool: ...


def focused_caret_anchor() -> ScreenAnchor | None:
    """Return the focused accessible text caret in screen coordinates."""

    try:
        if Atspi.get_desktop_count() <= 0:
            return None
        desktop = Atspi.get_desktop(0)
        pending = [desktop]
        visited = 0
        while pending and visited < 4096:
            accessible = pending.pop()
            visited += 1
            state = accessible.get_state_set()
            text = accessible.get_text_iface()
            if state.contains(Atspi.StateType.FOCUSED) and text is not None:
                caret = max(0, int(text.get_caret_offset()))
                rectangle = text.get_character_extents(
                    max(0, caret - 1), Atspi.CoordType.SCREEN
                )
                x = int(rectangle.x + (rectangle.width if caret else 0))
                return ScreenAnchor(x, int(rectangle.y + rectangle.height))
            child_count = min(512, int(accessible.get_child_count()))
            for index in range(child_count - 1, -1, -1):
                child = accessible.get_child_at_index(index)
                if child is not None:
                    pending.append(child)
    except Exception:
        return None
    return None


class LearningPromptWindow(Gtk.Window):
    """Small keyboard-focused prompt positioned above the active caret."""

    def __init__(
        self,
        application: Gtk.Application,
        backend: PromptBackend,
        confirm: Callable[[LearningPrompt], bool],
        dismiss: Callable[[LearningPrompt], bool],
    ) -> None:
        super().__init__(application=application)
        self.backend = backend
        self.confirm = confirm
        self.dismiss = dismiss
        self.prompt: LearningPrompt | None = None
        self.anchor: ScreenAnchor | None = None
        self.set_title("Обучение KeySwitch")
        self.set_decorated(False)
        self.set_resizable(False)
        self.set_modal(False)
        self.set_hide_on_close(True)
        self.set_focusable(True)
        self.set_default_size(410, -1)
        self.add_css_class("learning-prompt")

        box = Gtk.Box(
            orientation=Gtk.Orientation.VERTICAL,
            spacing=5,
            margin_top=14,
            margin_bottom=14,
            margin_start=18,
            margin_end=18,
        )
        self.question = Gtk.Label(
            label="Добавить слово в правила переключения?",
            xalign=0,
        )
        self.question.add_css_class("heading")
        self.word = Gtk.Label(xalign=0)
        self.word.add_css_class("learning-prompt-word")
        self.hint = Gtk.Label(label="Enter - ДА    Esc - НЕТ", xalign=0)
        self.hint.add_css_class("dim-label")
        box.append(self.question)
        box.append(self.word)
        box.append(self.hint)
        self.set_child(box)

        controller = Gtk.EventControllerKey()
        controller.connect("key-pressed", self._on_key_pressed)
        self.add_controller(controller)
        self.connect("close-request", self._on_close_request)

    def show_prompt(self, prompt: LearningPrompt) -> None:
        fallback = self.backend.input_anchor()
        caret = focused_caret_anchor()
        self.anchor = (
            ScreenAnchor(caret.x, caret.y, fallback.window if fallback else None)
            if caret is not None
            else fallback
        )
        self.prompt = prompt
        self.word.set_text(f"{prompt.original}  →  {prompt.replacement}")
        self.present()
        self.grab_focus()
        GLib.idle_add(self._position_above_anchor)

    def hide_prompt(self) -> None:
        anchor = self.anchor
        self.prompt = None
        self.anchor = None
        self.set_visible(False)
        if anchor is not None:
            self.backend.restore_window(anchor.window)

    def _position_above_anchor(self) -> bool:
        anchor = self.anchor
        surface = self.get_surface()
        if anchor is None or surface is None:
            return GLib.SOURCE_REMOVE
        x11_surface = cast(GdkX11.X11Surface, surface)
        window = int(GdkX11.X11Surface.get_xid(x11_surface))
        x = anchor.x - self.get_width() // 2
        y = anchor.y - self.get_height() - 12
        self.backend.position_window(window, x, y)
        return GLib.SOURCE_REMOVE

    def _on_key_pressed(
        self,
        _controller: Gtk.EventControllerKey,
        keyval: int,
        _keycode: int,
        _state: Gdk.ModifierType,
    ) -> bool:
        prompt = self.prompt
        if prompt is None:
            return False
        if keyval in {Gdk.KEY_Return, Gdk.KEY_KP_Enter}:
            self.confirm(prompt)
            return True
        if keyval == Gdk.KEY_Escape:
            self.dismiss(prompt)
            return True
        self.dismiss(prompt)
        return False

    def _on_close_request(self, _window: Gtk.Window) -> bool:
        prompt = self.prompt
        if prompt is not None:
            self.dismiss(prompt)
        return True
