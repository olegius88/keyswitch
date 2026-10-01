"""Linux windows of the switching rules: the offer above the caret and the rule window."""

from __future__ import annotations

import os
from collections.abc import Callable, Iterable
from typing import Protocol, cast

import dbus
import gi

gi.require_version("Atspi", "2.0")
gi.require_version("Gdk", "4.0")
gi.require_version("GdkX11", "4.0")
gi.require_version("Gtk", "4.0")

from gi.repository import Atspi, Gdk, GdkX11, GLib, Gtk  # noqa: E402

from .backend import ScreenAnchor
from .engine import LearningPrompt
from .learning import RULE_ACTIONS, RULE_MATCHES, RuleAction, RuleMatch
from .rule_editor import (
    ACTION_LABELS,
    ACTION_TITLE,
    CASE_LABEL,
    CONDITION_LEAD,
    CONDITION_TITLE,
    EXTRA_LEAD,
    MATCH_LABELS,
    PATTERN_LABEL,
    PROMPT_HINT,
    PROMPT_TITLE,
    WINDOW_TITLE,
    RuleDraft,
    layout_hint,
    prompt_word_line,
)
from .constants.geometry import CENTERING_DIVISOR
from .constants.learning_prompt import (
    LEARNING_PROMPT_ANCHOR_GAP_PIXELS,
    LEARNING_PROMPT_GTK_MARGIN_X_PIXELS,
    LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS,
    LEARNING_PROMPT_GTK_SPACING_PIXELS,
    LEARNING_PROMPT_GTK_WIDTH_PIXELS,
    MAX_CARET_SEARCH_CHILDREN,
    MAX_CARET_SEARCH_NODES,
)


ACCESSIBILITY_BUS_NAME = "org.a11y.Bus"


class PromptBackend(Protocol):
    def input_anchor(self) -> ScreenAnchor | None: ...

    def position_window(self, window: int, x: int, y: int) -> bool: ...

    def restore_window(self, window: int | None) -> bool: ...


class AccessibilityBusProbe(Protocol):
    def name_has_owner(self, name: str) -> bool: ...

    def list_activatable_names(self) -> Iterable[object]: ...


def focused_caret_anchor() -> ScreenAnchor | None:
    """Return the focused accessible text caret in screen coordinates."""

    if not _accessibility_bus_available():
        return None
    try:
        if Atspi.get_desktop_count() <= 0:
            return None
        desktop = Atspi.get_desktop(0)
        pending = [desktop]
        visited = 0
        while pending and visited < MAX_CARET_SEARCH_NODES:
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
            child_count = min(MAX_CARET_SEARCH_CHILDREN, int(accessible.get_child_count()))
            for index in range(child_count - 1, -1, -1):
                child = accessible.get_child_at_index(index)
                if child is not None:
                    pending.append(child)
    except Exception:
        return None
    return None


def _accessibility_bus_available() -> bool:
    """Avoid fatal libatspi calls when the desktop accessibility bus is absent."""

    if os.environ.get("GTK_A11Y", "").strip().lower() == "none":
        return False
    try:
        bus = cast(AccessibilityBusProbe, dbus.SessionBus())
        if bool(bus.name_has_owner(ACCESSIBILITY_BUS_NAME)):
            return True
        return any(
            str(name) == ACCESSIBILITY_BUS_NAME
            for name in bus.list_activatable_names()
        )
    except Exception:
        return False


class LearningPromptWindow(Gtk.Window):
    """Small keyboard-focused rule offer positioned above the active caret."""

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
        self.set_default_size(LEARNING_PROMPT_GTK_WIDTH_PIXELS, -1)
        self.add_css_class("learning-prompt")

        box = Gtk.Box(
            orientation=Gtk.Orientation.VERTICAL,
            spacing=LEARNING_PROMPT_GTK_SPACING_PIXELS,
            margin_top=LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS,
            margin_bottom=LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS,
            margin_start=LEARNING_PROMPT_GTK_MARGIN_X_PIXELS,
            margin_end=LEARNING_PROMPT_GTK_MARGIN_X_PIXELS,
        )
        self.question = Gtk.Label(label=PROMPT_TITLE, xalign=0)
        self.question.add_css_class("heading")
        self.word = Gtk.Label(xalign=0)
        self.word.add_css_class("learning-prompt-word")
        self.hint = Gtk.Label(label=PROMPT_HINT, xalign=0)
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
        self.word.set_text(prompt_word_line(prompt))
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
        x = anchor.x - self.get_width() // CENTERING_DIVISOR
        y = anchor.y - self.get_height() - LEARNING_PROMPT_ANCHOR_GAP_PIXELS
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


class RuleEditorWindow(Gtk.Window):
    """The rule window: the letters, the condition, the case and the action.

    Enter in the letters field is OK and Escape is Cancel. ``focused`` follows the
    window gaining and losing the keyboard, so the engine leaves what is typed here
    alone and corrects typing elsewhere. ``accept`` turns the form into a rule and
    answers with an empty string, or with what to fix, which the window shows
    instead of closing.
    """

    def __init__(
        self,
        application: Gtk.Application,
        draft: RuleDraft,
        accept: Callable[[RuleDraft], str],
        closed: Callable[[], None],
        focused: Callable[[bool], None],
    ) -> None:
        super().__init__(application=application)
        self.draft = draft
        self.accept = accept
        self.closed = closed
        self.focused = focused
        self.set_title(WINDOW_TITLE)
        self.set_resizable(False)
        box = Gtk.Box(
            orientation=Gtk.Orientation.VERTICAL,
            spacing=LEARNING_PROMPT_GTK_SPACING_PIXELS,
            margin_top=LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS,
            margin_bottom=LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS,
            margin_start=LEARNING_PROMPT_GTK_MARGIN_X_PIXELS,
            margin_end=LEARNING_PROMPT_GTK_MARGIN_X_PIXELS,
        )
        box.append(Gtk.Label(label=PATTERN_LABEL, xalign=0))
        self.entry = Gtk.Entry(text=draft.pattern)
        self.entry.connect("changed", lambda _entry: self._pattern_changed())
        self.entry.connect("activate", lambda _entry: self.confirm())
        box.append(self.entry)
        self.hint = Gtk.Label(xalign=0)
        self.hint.add_css_class("dim-label")
        box.append(self.hint)

        condition = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=LEARNING_PROMPT_GTK_SPACING_PIXELS)
        condition.append(Gtk.Label(label=CONDITION_LEAD, xalign=0))
        self.matches: dict[RuleMatch, Gtk.CheckButton] = {}
        for match in RULE_MATCHES:
            button = Gtk.CheckButton(label=MATCH_LABELS[match])
            button.set_group(next(iter(self.matches.values()), None))
            self.matches[match] = button
            condition.append(button)
        self.matches[draft.match].set_active(True)
        condition.append(Gtk.Label(label=EXTRA_LEAD, xalign=0))
        self.case_sensitive = Gtk.CheckButton(label=CASE_LABEL, active=draft.case_sensitive)
        condition.append(self.case_sensitive)
        box.append(Gtk.Frame(label=CONDITION_TITLE, child=condition))

        action = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=LEARNING_PROMPT_GTK_SPACING_PIXELS)
        self.actions: dict[RuleAction, Gtk.CheckButton] = {}
        for value in RULE_ACTIONS:
            button = Gtk.CheckButton(label=ACTION_LABELS[value])
            button.set_group(next(iter(self.actions.values()), None))
            self.actions[value] = button
            action.append(button)
        self.actions[draft.action].set_active(True)
        box.append(Gtk.Frame(label=ACTION_TITLE, child=action))

        self.problem = Gtk.Label(xalign=0, wrap=True)
        self.problem.add_css_class("error")
        box.append(self.problem)
        buttons = Gtk.Box(spacing=LEARNING_PROMPT_GTK_SPACING_PIXELS, halign=Gtk.Align.END)
        cancel = Gtk.Button(label="Отмена")
        cancel.connect("clicked", lambda _button: self.cancel())
        ok = Gtk.Button(label="OK")
        ok.add_css_class("suggested-action")
        ok.connect("clicked", lambda _button: self.confirm())
        buttons.append(cancel)
        buttons.append(ok)
        box.append(buttons)
        self.set_child(box)

        controller = Gtk.EventControllerKey()
        controller.connect("key-pressed", self._on_key_pressed)
        self.add_controller(controller)
        self.connect("close-request", self._on_close_request)
        self.connect("notify::is-active", lambda window, _parameter: self.focused(window.is_active()))
        self._pattern_changed()

    def current_draft(self) -> RuleDraft:
        match = next(value for value, button in self.matches.items() if button.get_active())
        action = next(value for value, button in self.actions.items() if button.get_active())
        return RuleDraft(
            self.entry.get_text(), match, self.case_sensitive.get_active(), action,
            self.draft.application, self.draft.editing,
        )

    def confirm(self) -> None:
        problem = self.accept(self.current_draft())
        if problem:
            self.problem.set_text(problem)
            return
        self._close()

    def cancel(self) -> None:
        self._close()

    def _close(self) -> None:
        self.destroy()
        self.focused(False)
        self.closed()

    def _pattern_changed(self) -> None:
        self.hint.set_text(layout_hint(self.current_draft()))
        self.problem.set_text("")

    def _on_key_pressed(
        self,
        _controller: Gtk.EventControllerKey,
        keyval: int,
        _keycode: int,
        _state: Gdk.ModifierType,
    ) -> bool:
        if keyval == Gdk.KEY_Escape:
            self.cancel()
            return True
        return False

    def _on_close_request(self, _window: Gtk.Window) -> bool:
        self.cancel()
        return True
